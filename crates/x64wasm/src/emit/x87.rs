//! x87 (`translator.md`, "x87"): each [`Op::X87`] expands here. TOP, the
//! tags and FCW are read from the state block at run time; ST(i) is
//! R[(TOP + i) & 7] at `STATE_FPR + 16 * p` (significand at +0, sign and
//! exponent at +8).
//!
//! Every expansion has the same order:
//! 1. FSW; a pending unmasked exception (ES) exits (#MF);
//! 2. the operands and the stack-fault test (an empty operand, or a full
//!    register under a push). A stack fault with IE unmasked exits; masked,
//!    the result is the indefinite and the status IE | SF (C1 = 1 for an
//!    overflow);
//! 3. otherwise the helper (or the inline move), and an exit if its status
//!    has ES;
//! 4. only then the writes: memory, registers with their tags, pops, and
//!    FSW (exception flags and SF ORed in, C1 and the defined condition
//!    codes replaced, TOP) and the tag word.

use wasm_encoder::{BlockType, InstructionSink, ValType};

use super::{ChunkEmitter, Label, P_STATE, mem};
use crate::abi::{self, FpFunc};
use crate::ir::{Value, X87, X87Arith, X87Kind, X87Src, X87Unary};

/// FSW bits.
const IE: i32 = 0x01;
const SF: i32 = 0x40;
const ES: i32 = 0x80;
const C0: i32 = 0x100;
const C1: i32 = 0x200;
const C2: i32 = 0x400;
const C3: i32 = 0x4000;
const TOP_MASK: i32 = 0x3800;
/// The QNaN floating-point indefinite: significand, sign and exponent.
const INDEF_M: i64 = 0xC000_0000_0000_0000u64 as i64;
const INDEF_SE: i32 = 0xFFFF;
const EMPTY: i32 = 3;

/// Scratch locals of the x87 expansions.
#[derive(Clone, Copy)]
pub(super) struct X87Locals {
    top: u32,
    fsw: u32,
    ftw: u32,
    fcw: u32,
    /// The status of the operation (helper status, or the stack fault's).
    st: u32,
    p: u32,
    q: u32,
    ase: u32,
    bse: u32,
    rse: u32,
    am: u32,
    bm: u32,
    rm: u32,
    addr: u32,
}

impl X87Locals {
    /// i32 and i64 locals, in declaration order.
    pub(super) const I32: u32 = 10;
    pub(super) const I64: u32 = 4;
    /// Names for the name section, in local order.
    pub(super) const NAMES: [&'static str; 14] = [
        "x87_top", "x87_fsw", "x87_ftw", "x87_fcw", "x87_st", "x87_p", "x87_q", "x87_ase",
        "x87_bse", "x87_rse", "x87_am", "x87_bm", "x87_rm", "x87_addr",
    ];

    /// Locals starting at `base`: [`Self::I32`] i32s, then [`Self::I64`]
    /// i64s.
    pub(super) fn at(base: u32) -> Self {
        let i = |k| base + k;
        let l = |k| base + Self::I32 + k;
        X87Locals {
            top: i(0),
            fsw: i(1),
            ftw: i(2),
            fcw: i(3),
            st: i(4),
            p: i(5),
            q: i(6),
            ase: i(7),
            bse: i(8),
            rse: i(9),
            am: l(0),
            bm: l(1),
            rm: l(2),
            addr: l(3),
        }
    }

    pub(super) fn decls() -> [(u32, ValType); 2] {
        [(Self::I32, ValType::I32), (Self::I64, ValType::I64)]
    }
}

/// Pushes the address of R[p] (the state block plus 16 p; the memarg
/// offset adds `STATE_FPR`).
fn reg_ptr(f: &mut InstructionSink<'_>, p: u32) {
    f.local_get(P_STATE);
    f.local_get(p);
    f.i64_extend_i32_u();
    f.i64_const(4);
    f.i64_shl();
    f.i64_add();
}

/// Pushes the tag from the contents (as FSTENV reports it): exponent
/// 0x7FFF special; exponent 0: zero if the significand is 0, else special;
/// otherwise valid if the J bit is set, else special (unnormal).
fn tag_of(f: &mut InstructionSink<'_>, m: u32, se: u32) {
    f.i32_const(2);
    // exponent 0: 2 - (m == 0)
    f.i32_const(2);
    f.local_get(m);
    f.i64_eqz();
    f.i32_sub();
    // J clear: 2, else 0
    f.local_get(m);
    f.i64_const(0);
    f.i64_ge_s();
    f.i32_const(1);
    f.i32_shl();
    f.local_get(se);
    f.i32_const(0x7fff);
    f.i32_and();
    f.i32_eqz();
    f.select();
    f.local_get(se);
    f.i32_const(0x7fff);
    f.i32_and();
    f.i32_const(0x7fff);
    f.i32_eq();
    f.select();
}

impl ChunkEmitter<'_> {
    fn x87_locals(&self) -> X87Locals {
        self.l_x87.expect("chunk has x87 locals")
    }

    fn open_if(&mut self, f: &mut InstructionSink<'_>) {
        f.if_(BlockType::Empty);
        self.labels.push(Label::If);
    }

    fn close_if(&mut self, f: &mut InstructionSink<'_>) {
        f.end();
        self.labels.pop();
    }

    /// `if <i32 on the stack> { exit EXIT_UNSUPPORTED at the instruction }`
    fn exit_unsupported_if(&mut self, f: &mut InstructionSink<'_>) {
        self.open_if(f);
        self.exit(f, abi::EXIT_UNSUPPORTED, self.cur_addr);
        self.close_if(f);
    }

    /// `local = (TOP + i) & 7`
    fn x87_phys(&self, f: &mut InstructionSink<'_>, local: u32, i: i32) {
        let l = self.x87_locals();
        f.local_get(l.top);
        if i != 0 {
            f.i32_const(i);
            f.i32_add();
            f.i32_const(7);
            f.i32_and();
        }
        f.local_set(local);
    }

    /// Pushes 1 if R[p] is empty.
    fn x87_empty(&self, f: &mut InstructionSink<'_>, p: u32) {
        let l = self.x87_locals();
        f.local_get(l.ftw);
        f.local_get(p);
        f.i32_const(1);
        f.i32_shl();
        f.i32_shr_u();
        f.i32_const(3);
        f.i32_and();
        f.i32_const(EMPTY);
        f.i32_eq();
    }

    /// Sets R[p]'s tag to the value `tag` pushes.
    fn x87_set_tag(
        &self,
        f: &mut InstructionSink<'_>,
        p: u32,
        tag: impl FnOnce(&mut InstructionSink<'_>),
    ) {
        let l = self.x87_locals();
        f.local_get(l.ftw);
        f.i32_const(3);
        f.local_get(p);
        f.i32_const(1);
        f.i32_shl();
        f.i32_shl();
        f.i32_const(-1);
        f.i32_xor();
        f.i32_and();
        tag(f);
        f.local_get(p);
        f.i32_const(1);
        f.i32_shl();
        f.i32_shl();
        f.i32_or();
        f.local_set(l.ftw);
    }

    /// `m, se = R[p]`
    fn x87_read(&self, f: &mut InstructionSink<'_>, p: u32, m: u32, se: u32) {
        reg_ptr(f, p);
        f.i64_load(mem(abi::STATE_FPR));
        f.local_set(m);
        reg_ptr(f, p);
        f.i32_load16_u(mem(abi::STATE_FPR + 8));
        f.local_set(se);
    }

    /// `R[p] = m, se`, with the tag from the contents.
    fn x87_write(&self, f: &mut InstructionSink<'_>, p: u32, m: u32, se: u32) {
        reg_ptr(f, p);
        f.local_get(m);
        f.i64_store(mem(abi::STATE_FPR));
        reg_ptr(f, p);
        f.local_get(se);
        f.i32_store16(mem(abi::STATE_FPR + 8));
        self.x87_set_tag(f, p, |f| tag_of(f, m, se));
    }

    /// The stack-fault branch: exits if IE is unmasked; otherwise the
    /// result is the indefinite (`rm` = `indef`, `rse`) and the status
    /// IE | SF | `cc`.
    fn x87_stack_fault(&mut self, f: &mut InstructionSink<'_>, indef: i64, cc: i32) {
        let l = self.x87_locals();
        f.local_get(l.fcw);
        f.i32_const(IE);
        f.i32_and();
        f.i32_eqz();
        self.exit_unsupported_if(f);
        f.i64_const(indef);
        f.local_set(l.rm);
        f.i32_const(INDEF_SE);
        f.local_set(l.rse);
        f.i32_const(IE | SF | cc);
        f.local_set(l.st);
    }

    /// After a helper call: `st = status()`; exits if ES is set.
    fn x87_status(&mut self, f: &mut InstructionSink<'_>) {
        let l = self.x87_locals();
        f.call(self.fp.index(FpFunc::Status));
        f.local_tee(l.st);
        f.i32_const(ES);
        f.i32_and();
        self.exit_unsupported_if(f);
    }

    /// `rse = st >> 16` (the result's sign and exponent from the status).
    fn x87_status_se(&self, f: &mut InstructionSink<'_>) {
        let l = self.x87_locals();
        f.local_get(l.st);
        f.i32_const(16);
        f.i32_shr_u();
        f.local_set(l.rse);
    }

    /// Marks ST(0) empty and increments TOP.
    fn x87_pop(&self, f: &mut InstructionSink<'_>) {
        let l = self.x87_locals();
        self.x87_set_tag(f, l.top, |f| {
            f.i32_const(EMPTY);
        });
        f.local_get(l.top);
        f.i32_const(1);
        f.i32_add();
        f.i32_const(7);
        f.i32_and();
        f.local_set(l.top);
    }

    /// Stores FSW and the tag word: the status's exception flags and SF
    /// ORed in, the condition codes in `cc` replaced by the status's, TOP.
    fn x87_finish(&self, f: &mut InstructionSink<'_>, cc: i32) {
        let l = self.x87_locals();
        f.local_get(P_STATE);
        f.local_get(l.fsw);
        f.i32_const(!(TOP_MASK | cc));
        f.i32_and();
        f.local_get(l.st);
        f.i32_const(0x7f | cc);
        f.i32_and();
        f.i32_or();
        f.local_get(l.top);
        f.i32_const(11);
        f.i32_shl();
        f.i32_or();
        f.i32_store16(mem(abi::STATE_FSW));
        f.local_get(P_STATE);
        f.local_get(l.ftw);
        f.i32_store16(mem(abi::STATE_FTW));
    }

    /// `bm` = the memory operand of `kind` (integers sign-extended, floats
    /// as bits; f80: `bm` and `bse`).
    fn x87_load_mem(&self, f: &mut InstructionSink<'_>, kind: X87Kind) {
        let l = self.x87_locals();
        f.local_get(l.addr);
        match kind {
            X87Kind::F32 => f.i64_load32_u(mem(0)),
            X87Kind::F64 | X87Kind::I64 | X87Kind::F80 => f.i64_load(mem(0)),
            X87Kind::I16 => f.i64_load16_s(mem(0)),
            X87Kind::I32 => f.i64_load32_s(mem(0)),
        };
        f.local_set(l.bm);
        if kind == X87Kind::F80 {
            f.local_get(l.addr);
            f.i32_load16_u(mem(8));
            f.local_set(l.bse);
        }
    }

    /// Stores `rm` (and for f80 `rse`) to the memory operand.
    fn x87_store_mem(&self, f: &mut InstructionSink<'_>, kind: X87Kind) {
        let l = self.x87_locals();
        f.local_get(l.addr);
        f.local_get(l.rm);
        match kind {
            X87Kind::F32 | X87Kind::I32 => f.i64_store32(mem(0)),
            X87Kind::F64 | X87Kind::I64 | X87Kind::F80 => f.i64_store(mem(0)),
            X87Kind::I16 => f.i64_store16(mem(0)),
        };
        if kind == X87Kind::F80 {
            f.local_get(l.addr);
            f.local_get(l.rse);
            f.i32_store16(mem(8));
        }
    }

    /// The source operand into `bm`, `bse` (and `q` = its register):
    /// returns whether it is a register (that can be empty).
    fn x87_src(&self, f: &mut InstructionSink<'_>, src: X87Src) -> bool {
        let l = self.x87_locals();
        match src {
            X87Src::St(i) => {
                self.x87_phys(f, l.q, i as i32);
                self.x87_read(f, l.q, l.bm, l.bse);
                true
            }
            X87Src::Mem(kind) => {
                self.x87_load_mem(f, kind);
                if kind != X87Kind::F80 {
                    f.i32_const(0);
                    f.local_set(l.bse);
                }
                false
            }
        }
    }

    pub(super) fn x87_op(&mut self, f: &mut InstructionSink<'_>, op: X87, addr: Option<Value>) {
        let l = self.x87_locals();
        if let Some(a) = addr {
            self.value(f, a);
            f.local_set(l.addr);
        }
        // A pending unmasked exception: #MF.
        f.local_get(P_STATE);
        f.i32_load16_u(mem(abi::STATE_FSW));
        f.local_tee(l.fsw);
        f.i32_const(ES);
        f.i32_and();
        self.exit_unsupported_if(f);
        if op == X87::Wait {
            return;
        }
        if op == X87::LoadEnv {
            self.x87_load_env(f);
            return;
        }
        f.local_get(l.fsw);
        f.i32_const(11);
        f.i32_shr_u();
        f.i32_const(7);
        f.i32_and();
        f.local_set(l.top);
        f.local_get(P_STATE);
        f.i32_load16_u(mem(abi::STATE_FTW));
        f.local_set(l.ftw);
        f.local_get(P_STATE);
        f.i32_load16_u(mem(abi::STATE_FCW));
        f.local_set(l.fcw);
        match op {
            X87::Wait | X87::LoadEnv => unreachable!(),
            X87::Load(_) | X87::LoadSt(_) | X87::LoadConst(..) => self.x87_push(f, op),
            X87::Store { kind, pop } => self.x87_store(f, kind, pop),
            X87::StoreSt { i, pop } => {
                self.x87_phys(f, l.q, i as i32);
                self.x87_empty(f, l.top);
                self.open_if(f);
                self.x87_stack_fault(f, INDEF_M, 0);
                f.else_();
                self.x87_read(f, l.top, l.rm, l.rse);
                f.i32_const(0);
                f.local_set(l.st);
                self.close_if(f);
                self.x87_write(f, l.q, l.rm, l.rse);
                if pop {
                    self.x87_pop(f);
                }
                self.x87_finish(f, C1);
            }
            X87::Xch(i) => {
                self.x87_phys(f, l.q, i as i32);
                f.i32_const(0);
                f.local_set(l.st);
                // An empty register reads as the indefinite.
                for (p, m, se) in [(l.top, l.am, l.ase), (l.q, l.bm, l.bse)] {
                    self.x87_read(f, p, m, se);
                    self.x87_empty(f, p);
                    self.open_if(f);
                    self.x87_stack_fault(f, INDEF_M, 0);
                    f.i64_const(INDEF_M);
                    f.local_set(m);
                    f.i32_const(INDEF_SE);
                    f.local_set(se);
                    self.close_if(f);
                }
                self.x87_write(f, l.top, l.bm, l.bse);
                self.x87_write(f, l.q, l.am, l.ase);
                self.x87_finish(f, C1);
            }
            X87::Arith { op, dst, src, pop } => {
                self.x87_phys(f, l.p, dst as i32);
                let reg = self.x87_src(f, src);
                self.x87_empty(f, l.p);
                if reg {
                    self.x87_empty(f, l.q);
                    f.i32_or();
                }
                self.open_if(f);
                self.x87_stack_fault(f, INDEF_M, 0);
                f.else_();
                self.x87_read(f, l.p, l.am, l.ase);
                f.local_get(l.am);
                f.local_get(l.ase);
                f.local_get(l.bm);
                f.local_get(l.bse);
                f.i32_const(src_kind(src));
                f.local_get(l.fcw);
                let func = match op {
                    X87Arith::Add => FpFunc::F80Add,
                    X87Arith::Sub => FpFunc::F80Sub,
                    X87Arith::Subr => FpFunc::F80Subr,
                    X87Arith::Mul => FpFunc::F80Mul,
                    X87Arith::Div => FpFunc::F80Div,
                    X87Arith::Divr => FpFunc::F80Divr,
                };
                f.call(self.fp.index(func));
                f.local_set(l.rm);
                self.x87_status(f);
                self.x87_status_se(f);
                self.close_if(f);
                self.x87_write(f, l.p, l.rm, l.rse);
                if pop {
                    self.x87_pop(f);
                }
                self.x87_finish(f, C1);
            }
            X87::Unary(u) => {
                self.x87_empty(f, l.top);
                self.open_if(f);
                self.x87_stack_fault(f, INDEF_M, 0);
                f.else_();
                self.x87_read(f, l.top, l.am, l.ase);
                match u {
                    X87Unary::Sqrt | X87Unary::Round => {
                        f.local_get(l.am);
                        f.local_get(l.ase);
                        f.local_get(l.fcw);
                        let func = match u {
                            X87Unary::Sqrt => FpFunc::F80Sqrt,
                            _ => FpFunc::F80Round,
                        };
                        f.call(self.fp.index(func));
                        f.local_set(l.rm);
                        self.x87_status(f);
                        self.x87_status_se(f);
                    }
                    X87Unary::Chs | X87Unary::Abs => {
                        f.local_get(l.am);
                        f.local_set(l.rm);
                        f.local_get(l.ase);
                        if u == X87Unary::Chs {
                            f.i32_const(0x8000);
                            f.i32_xor();
                        } else {
                            f.i32_const(0x7fff);
                            f.i32_and();
                        }
                        f.local_set(l.rse);
                        f.i32_const(0);
                        f.local_set(l.st);
                    }
                }
                self.close_if(f);
                self.x87_write(f, l.top, l.rm, l.rse);
                self.x87_finish(f, C1);
            }
            X87::Scale => {
                self.x87_src(f, X87Src::St(1));
                self.x87_empty(f, l.top);
                self.x87_empty(f, l.q);
                f.i32_or();
                self.open_if(f);
                self.x87_stack_fault(f, INDEF_M, 0);
                f.else_();
                self.x87_read(f, l.top, l.am, l.ase);
                f.local_get(l.am);
                f.local_get(l.ase);
                f.local_get(l.bm);
                f.local_get(l.bse);
                f.local_get(l.fcw);
                f.call(self.fp.index(FpFunc::F80Scale));
                f.local_set(l.rm);
                self.x87_status(f);
                self.x87_status_se(f);
                self.close_if(f);
                self.x87_write(f, l.top, l.rm, l.rse);
                self.x87_finish(f, C1);
            }
            X87::Com { src, quiet, pops } => {
                self.x87_compare(f, src, quiet, false);
                for _ in 0..pops {
                    self.x87_pop(f);
                }
                self.x87_finish(f, C3 | C2 | C1 | C0);
            }
            X87::Comi { dst, i, quiet, pop } => {
                // C1 stays unless the stack faults (then 0), as the CPU does.
                self.x87_compare(f, X87Src::St(i), quiet, true);
                if pop {
                    self.x87_pop(f);
                }
                // C0, C2 and C3 stay; C1 = 0.
                self.x87_finish(f, C1);
                // C3 -> ZF (0x40), C2 -> PF (0x04), C0 -> CF (0x01).
                f.local_get(l.st);
                f.i32_const(8);
                f.i32_shr_u();
                f.i32_const(0x45);
                f.i32_and();
                f.i64_extend_i32_u();
                f.local_set(self.temp(dst));
            }
        }
    }

    /// fld, fild, fldz …: TOP - 1; a full new ST(0) is an overflow (C1 =
    /// 1), an empty ST(i) source an underflow (C1 = 0).
    fn x87_push(&mut self, f: &mut InstructionSink<'_>, op: X87) {
        let l = self.x87_locals();
        if let X87::Load(kind) = op {
            self.x87_load_mem(f, kind);
        }
        self.x87_phys(f, l.p, -1);
        // Overflow: C1 = 1.
        self.x87_empty(f, l.p);
        f.i32_eqz();
        f.local_tee(l.rse);
        if let X87::LoadSt(i) = op {
            self.x87_phys(f, l.q, i as i32);
            self.x87_empty(f, l.q);
            f.i32_or();
        }
        self.open_if(f);
        {
            // rse holds the overflow bit until the fault branch replaces it.
            f.local_get(l.rse);
            f.local_set(l.ase);
            self.x87_stack_fault(f, INDEF_M, 0);
            f.local_get(l.st);
            f.local_get(l.ase);
            f.i32_const(9);
            f.i32_shl();
            f.i32_or();
            f.local_set(l.st);
        }
        f.else_();
        match op {
            X87::Load(X87Kind::F80) => {
                f.local_get(l.bm);
                f.local_set(l.rm);
                f.local_get(l.bse);
                f.local_set(l.rse);
                f.i32_const(0);
                f.local_set(l.st);
            }
            X87::Load(kind) => {
                f.local_get(l.bm);
                f.i32_const(kind as i32);
                f.local_get(l.fcw);
                f.call(self.fp.index(FpFunc::F80Load));
                f.local_set(l.rm);
                self.x87_status(f);
                self.x87_status_se(f);
            }
            X87::LoadSt(_) => {
                self.x87_read(f, l.q, l.rm, l.rse);
                f.i32_const(0);
                f.local_set(l.st);
            }
            X87::LoadConst(m, se) => {
                f.i64_const(m as i64);
                f.local_set(l.rm);
                f.i32_const(se as i32);
                f.local_set(l.rse);
                f.i32_const(0);
                f.local_set(l.st);
            }
            _ => unreachable!(),
        }
        self.close_if(f);
        self.x87_write(f, l.p, l.rm, l.rse);
        f.local_get(l.p);
        f.local_set(l.top);
        self.x87_finish(f, C1);
    }

    /// fst(p) m, fist(p) m: an empty ST(0) stores the format's indefinite.
    fn x87_store(&mut self, f: &mut InstructionSink<'_>, kind: X87Kind, pop: bool) {
        let l = self.x87_locals();
        self.x87_empty(f, l.top);
        self.open_if(f);
        self.x87_stack_fault(f, kind.indefinite() as i64, 0);
        f.else_();
        self.x87_read(f, l.top, l.am, l.ase);
        if kind == X87Kind::F80 {
            f.local_get(l.am);
            f.local_set(l.rm);
            f.local_get(l.ase);
            f.local_set(l.rse);
            f.i32_const(0);
            f.local_set(l.st);
        } else {
            f.local_get(l.am);
            f.local_get(l.ase);
            f.i32_const(kind as i32);
            f.local_get(l.fcw);
            f.call(self.fp.index(FpFunc::F80Store));
            f.local_set(l.rm);
            self.x87_status(f);
        }
        self.close_if(f);
        self.x87_store_mem(f, kind);
        if pop {
            self.x87_pop(f);
        }
        self.x87_finish(f, C1);
    }

    /// The compare of ST(0) with `src` into `st` (C3, C2, C0; unordered
    /// on a stack fault). With `keep_c1`, `st` carries FSW's C1 unless the
    /// stack faults. Writes nothing.
    fn x87_compare(
        &mut self,
        f: &mut InstructionSink<'_>,
        src: X87Src,
        quiet: bool,
        keep_c1: bool,
    ) {
        let l = self.x87_locals();
        let reg = self.x87_src(f, src);
        self.x87_empty(f, l.top);
        if reg {
            self.x87_empty(f, l.q);
            f.i32_or();
        }
        self.open_if(f);
        self.x87_stack_fault(f, INDEF_M, C3 | C2 | C0);
        f.else_();
        self.x87_read(f, l.top, l.am, l.ase);
        f.local_get(l.am);
        f.local_get(l.ase);
        f.local_get(l.bm);
        f.local_get(l.bse);
        f.i32_const(src_kind(src));
        f.i32_const(quiet as i32);
        f.local_get(l.fcw);
        f.call(self.fp.index(FpFunc::F80Cmp));
        f.drop();
        self.x87_status(f);
        if keep_c1 {
            f.local_get(l.st);
            f.local_get(l.fsw);
            f.i32_const(C1);
            f.i32_and();
            f.i32_or();
            f.local_set(l.st);
        }
        self.close_if(f);
    }

    /// fldenv m28: FCW, FSW and the tag word from memory; a register the
    /// image marks non-empty gets its tag from its contents.
    fn x87_load_env(&mut self, f: &mut InstructionSink<'_>) {
        let l = self.x87_locals();
        // The image's tag word.
        f.local_get(l.addr);
        f.i32_load16_u(mem(8));
        f.local_set(l.q);
        f.i32_const(0);
        f.local_set(l.ftw);
        for p in 0..8u32 {
            f.local_get(P_STATE);
            f.i64_load(mem(abi::STATE_FPR + 16 * p as u64));
            f.local_set(l.am);
            f.local_get(P_STATE);
            f.i32_load16_u(mem(abi::STATE_FPR + 16 * p as u64 + 8));
            f.local_set(l.ase);
            f.local_get(l.ftw);
            f.i32_const(EMPTY);
            tag_of(f, l.am, l.ase);
            f.local_get(l.q);
            f.i32_const(2 * p as i32);
            f.i32_shr_u();
            f.i32_const(3);
            f.i32_and();
            f.i32_const(EMPTY);
            f.i32_eq();
            f.select();
            f.i32_const(2 * p as i32);
            f.i32_shl();
            f.i32_or();
            f.local_set(l.ftw);
        }
        f.local_get(P_STATE);
        f.local_get(l.addr);
        f.i32_load16_u(mem(0));
        f.i32_store16(mem(abi::STATE_FCW));
        f.local_get(P_STATE);
        f.local_get(l.addr);
        f.i32_load16_u(mem(4));
        f.i32_store16(mem(abi::STATE_FSW));
        f.local_get(P_STATE);
        f.local_get(l.ftw);
        f.i32_store16(mem(abi::STATE_FTW));
    }
}

/// The helper's `bkind` for a source operand.
fn src_kind(src: X87Src) -> i32 {
    match src {
        X87Src::St(_) => X87Kind::F80 as i32,
        X87Src::Mem(k) => k as i32,
    }
}
