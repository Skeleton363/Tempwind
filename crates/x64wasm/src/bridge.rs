//! The bridge module of a Wine process (`docs/design/abi.md` v5, "Wine
//! boundary", "The bridge"): JS instantiates it once per process, before
//! the Unix side, which imports its exports. It holds no guest code.
//!
//! - Imports: `env.memory` (memory64, shared, as a shared translated module
//!   declares it), `env.table` (minimum [`abi::TABLE_RESERVED`]) and
//!   `env.depth`.
//! - Exports: `run` (through `env.table[RUN_INDEX]`), `depth_get`,
//!   `depth_set` and `flags_materialize` (the trampoline's step 3, emitted by
//!   the same code as every translated module's `run`).
//! - An active element segment puts `left_code` at [`abi::RUN_INDEX`] and
//!   `tombstone` at [`abi::TOMBSTONE_INDEX`].

use std::borrow::Cow;

use wasm_encoder::{
    CodeSection, ConstExpr, ElementSection, Elements, EntityType, ExportKind, ExportSection,
    Function, FunctionSection, GlobalType, ImportSection, MemoryType, Module, NameMap, NameSection,
    RefType, TableType, TypeSection, ValType,
};

use crate::abi;
use crate::emit::{emit_flags, materialize_rflags, mem};
use crate::ir::CC_EAGER;

// Types.
/// `(state i64) -> i32`: `run` and the placeholder `left_code`.
const T_RUN: u32 = 0;
/// `flags(op i32, a i64, b i64, r i64) -> i64`.
const T_FLAGS: u32 = 1;
/// The guest-function type `(state i64, entry i32, ret i64) -> i32`.
const T_GUEST: u32 = 2;
const T_DEPTH_GET: u32 = 3;
const T_DEPTH_SET: u32 = 4;
/// `(state i64) -> ()`.
const T_MATERIALIZE: u32 = 5;

// Functions (the bridge imports none).
const F_RUN: u32 = 0;
const F_DEPTH_GET: u32 = 1;
const F_DEPTH_SET: u32 = 2;
const F_FLAGS_MATERIALIZE: u32 = 3;
const F_FLAGS: u32 = 4;
const F_LEFT_CODE: u32 = 5;
const F_TOMBSTONE: u32 = 6;

const TABLE: u32 = 0;
const G_DEPTH: u32 = 0;

/// The bridge module (`x64wasm-cli bridge --out FILE`).
pub fn module() -> Vec<u8> {
    let mut module = Module::new();

    let mut types = TypeSection::new();
    types.ty().function([ValType::I64], [ValType::I32]); // T_RUN
    types.ty().function(
        [ValType::I32, ValType::I64, ValType::I64, ValType::I64],
        [ValType::I64],
    ); // T_FLAGS
    types
        .ty()
        .function([ValType::I64, ValType::I32, ValType::I64], [ValType::I32]); // T_GUEST
    types.ty().function([], [ValType::I32]); // T_DEPTH_GET
    types.ty().function([ValType::I32], []); // T_DEPTH_SET
    types.ty().function([ValType::I64], []); // T_MATERIALIZE
    module.section(&types);

    let mut imports = ImportSection::new();
    imports.import(
        abi::MEMORY_MODULE,
        abi::MEMORY_NAME,
        EntityType::Memory(MemoryType {
            minimum: 1,
            maximum: Some(abi::MAX_MEMORY_PAGES),
            memory64: true,
            shared: true,
            page_size_log2: None,
        }),
    );
    imports.import(
        abi::MEMORY_MODULE,
        abi::TABLE_NAME,
        EntityType::Table(TableType {
            element_type: RefType::FUNCREF,
            table64: false,
            minimum: abi::TABLE_RESERVED.into(),
            maximum: None,
            shared: false,
        }),
    );
    imports.import(
        abi::MEMORY_MODULE,
        abi::DEPTH_NAME,
        EntityType::Global(GlobalType {
            val_type: ValType::I32,
            mutable: true,
            shared: false,
        }),
    );
    module.section(&imports);

    let funcs_in_order = [
        (F_RUN, T_RUN, abi::RUN_EXPORT),
        (F_DEPTH_GET, T_DEPTH_GET, abi::BRIDGE_DEPTH_GET_EXPORT),
        (F_DEPTH_SET, T_DEPTH_SET, abi::BRIDGE_DEPTH_SET_EXPORT),
        (
            F_FLAGS_MATERIALIZE,
            T_MATERIALIZE,
            abi::BRIDGE_FLAGS_MATERIALIZE_EXPORT,
        ),
        (F_FLAGS, T_FLAGS, "flags"),
        (F_LEFT_CODE, T_RUN, "left_code"),
        (F_TOMBSTONE, T_GUEST, "tombstone"),
    ];
    let mut funcs = FunctionSection::new();
    for (i, &(f, ty, _)) in funcs_in_order.iter().enumerate() {
        debug_assert_eq!(f, i as u32);
        funcs.function(ty);
    }
    module.section(&funcs);

    let mut exports = ExportSection::new();
    for &(f, _, name) in &funcs_in_order[..=F_FLAGS_MATERIALIZE as usize] {
        exports.export(name, ExportKind::Func, f);
    }
    module.section(&exports);

    let mut elements = ElementSection::new();
    for (index, func) in [
        (abi::RUN_INDEX, F_LEFT_CODE),
        (abi::TOMBSTONE_INDEX, F_TOMBSTONE),
    ] {
        elements.active(
            Some(TABLE),
            &ConstExpr::i32_const(index as i32),
            Elements::Functions(Cow::Borrowed(&[func])),
        );
    }
    module.section(&elements);

    let mut code = CodeSection::new();
    code.function(&emit_run());
    code.function(&emit_depth_get());
    code.function(&emit_depth_set());
    code.function(&emit_flags_materialize());
    code.function(&emit_flags());
    code.function(&emit_left_code());
    code.function(&emit_tombstone());
    module.section(&code);

    let mut names = NameSection::new();
    let mut fnames = NameMap::new();
    for &(f, _, name) in &funcs_in_order {
        fnames.append(f, name);
    }
    names.functions(&fnames);
    let mut tables = NameMap::new();
    tables.append(TABLE, abi::TABLE_NAME);
    names.tables(&tables);
    let mut globals = NameMap::new();
    globals.append(G_DEPTH, abi::DEPTH_NAME);
    names.globals(&globals);
    module.section(&names);
    module.finish()
}

/// `run(state) -> i32`: a tail call of type `(i64) -> i32` through
/// `env.table[RUN_INDEX]`.
fn emit_run() -> Function {
    let mut func = Function::new([]);
    let mut f = func.instructions();
    f.local_get(0);
    f.i32_const(abi::RUN_INDEX as i32);
    f.return_call_indirect(TABLE, T_RUN);
    f.end();
    func
}

fn emit_depth_get() -> Function {
    let mut func = Function::new([]);
    let mut f = func.instructions();
    f.global_get(G_DEPTH);
    f.end();
    func
}

fn emit_depth_set() -> Function {
    let mut func = Function::new([]);
    let mut f = func.instructions();
    f.local_get(0);
    f.global_set(G_DEPTH);
    f.end();
    func
}

/// `flags_materialize(state)`: the trampoline's step 3, then the record
/// holds the materialized flags (`cc_op = 0`, `cc_a = rflags`).
fn emit_flags_materialize() -> Function {
    const STATE: u32 = 0;
    let mut func = Function::new([]);
    let mut f = func.instructions();
    materialize_rflags(&mut f, STATE, F_FLAGS);
    f.local_get(STATE);
    f.i32_const(CC_EAGER as i32);
    f.i32_store(mem(abi::STATE_CC_OP));
    f.local_get(STATE);
    f.local_get(STATE);
    f.i64_load(mem(abi::STATE_RFLAGS));
    f.i64_store(mem(abi::STATE_CC_A));
    f.end();
    func
}

/// `left_code(state) -> i32`: `state.exit = EXIT_LEFT_CODE`, returned;
/// nothing else changes, so the Unix side's loop translates `state.rip`.
fn emit_left_code() -> Function {
    let mut func = Function::new([]);
    let mut f = func.instructions();
    f.local_get(0);
    f.i32_const(abi::EXIT_LEFT_CODE as i32);
    f.i32_store(mem(abi::STATE_EXIT));
    f.i32_const(abi::EXIT_LEFT_CODE as i32);
    f.end();
    func
}

/// `tombstone(state, entry, ret) -> i32`, the function removed lookup slots
/// name: `state.rip = LOOKUP_TOMBSTONE`, `state.exit = EXIT_UNSUPPORTED`,
/// and `STATUS_EXIT`.
fn emit_tombstone() -> Function {
    let mut func = Function::new([]);
    let mut f = func.instructions();
    f.local_get(0);
    f.i64_const(abi::LOOKUP_TOMBSTONE as i64);
    f.i64_store(mem(abi::STATE_RIP));
    f.local_get(0);
    f.i32_const(abi::EXIT_UNSUPPORTED as i32);
    f.i32_store(mem(abi::STATE_EXIT));
    f.i32_const(abi::STATUS_EXIT);
    f.end();
    func
}
