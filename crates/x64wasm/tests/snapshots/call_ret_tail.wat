(module
  (type (;0;) (func (param i64) (result i32)))
  (type (;1;) (func (param i32 i64 i64 i64) (result i64)))
  (type (;2;) (func (param i64 i32 i64) (result i32)))
  (type (;3;) (func (param i64) (result i32 i32)))
  (import "env" "memory" (memory (;0;) i64 1))
  (table $chunks (;0;) 3 3 funcref)
  (global $depth (;0;) (mut i32) i32.const 0)
  (export "run" (func $run))
  (elem (;0;) (table $chunks) (i32.const 0) func $fn_0x1000 $fn_0x100b $fn_0x1011)
  (func $run (;0;) (type 0) (param $state i64) (result i32)
    (local $func i32) (local $entry i32) (local $status i32)
    local.get $state
    i32.const 0
    i32.store offset=148 align=1
    local.get $state
    local.get $state
    i64.load offset=136 align=1
    i64.store offset=152 align=1
    i32.const 0
    global.set $depth
    block ;; label = @1
      loop ;; label = @2
        local.get $state
        i64.load offset=128 align=1
        call $lookup
        local.set $entry
        local.tee $func
        i32.const 0
        i32.lt_s
        if ;; label = @3
          i32.const 1
          local.set $status
          br 2 (;@1;)
        end
        local.get $state
        local.get $entry
        i64.const -1
        local.get $func
        call_indirect (type 2)
        local.tee $status
        i32.const 2
        i32.eq
        if ;; label = @3
          local.get $state
          i32.load offset=144 align=1
          local.set $status
          br 2 (;@1;)
        end
        local.get $status
        i32.eqz
        if ;; label = @3
          local.get $state
          i64.const -1
          i64.store offset=128 align=1
        end
        i32.const 0
        global.set $depth
        br 0 (;@2;)
      end
    end
    local.get $state
    local.get $state
    i64.load offset=136 align=1
    i64.const -2262
    i64.and
    local.get $state
    i32.load offset=148 align=1
    local.get $state
    i64.load offset=152 align=1
    local.get $state
    i64.load offset=160 align=1
    local.get $state
    i64.load offset=168 align=1
    call $flags
    i64.or
    i64.store offset=136 align=1
    local.get $state
    local.get $status
    i32.store offset=144 align=1
    local.get $status
  )
  (func $lookup (;1;) (type 3) (param $rip i64) (result i32 i32)
    local.get $rip
    i64.const 4106
    i64.lt_u
    if ;; label = @1
      local.get $rip
      i64.const 4096
      i64.eq
      if ;; label = @2
        i32.const 0
        i32.const 0
        return
      end
      local.get $rip
      i64.const 4101
      i64.eq
      if ;; label = @2
        i32.const 0
        i32.const 1
        return
      end
    else
      local.get $rip
      i64.const 4106
      i64.eq
      if ;; label = @2
        i32.const 0
        i32.const 2
        return
      end
      local.get $rip
      i64.const 4107
      i64.eq
      if ;; label = @2
        i32.const 1
        i32.const 0
        return
      end
      local.get $rip
      i64.const 4113
      i64.eq
      if ;; label = @2
        i32.const 2
        i32.const 0
        return
      end
    end
    i32.const -1
    i32.const 0
  )
  (func $flags (;2;) (type 1) (param $op i32) (param $a i64) (param $b i64) (param $r i64) (result i64)
    (local $sign i64) (local $cf_of i64)
    local.get $op
    i32.eqz
    if ;; label = @1
      local.get $a
      i64.const 2261
      i64.and
      return
    end
    i64.const 1
    i64.const 8
    local.get $op
    i32.const 1
    i32.sub
    i32.const 3
    i32.and
    i64.extend_i32_u
    i64.shl
    i64.const 1
    i64.sub
    i64.shl
    local.set $sign
    local.get $op
    i32.const 1
    i32.sub
    i32.const 2
    i32.shr_u
    if (result i64) ;; label = @1
      local.get $a
      local.get $b
      i64.lt_u
      i64.extend_i32_u
      local.get $a
      local.get $b
      i64.xor
      local.get $a
      local.get $r
      i64.xor
      i64.and
      local.get $sign
      i64.and
      i64.eqz
      i32.eqz
      i64.extend_i32_u
      i64.const 11
      i64.shl
      i64.or
    else
      local.get $r
      local.get $a
      i64.lt_u
      i64.extend_i32_u
      local.get $a
      local.get $r
      i64.xor
      local.get $b
      local.get $r
      i64.xor
      i64.and
      local.get $sign
      i64.and
      i64.eqz
      i32.eqz
      i64.extend_i32_u
      i64.const 11
      i64.shl
      i64.or
    end
    local.set $cf_of
    local.get $r
    i64.const 255
    i64.and
    i64.popcnt
    i64.const 1
    i64.and
    i64.const 1
    i64.xor
    i64.const 2
    i64.shl
    local.get $a
    local.get $b
    i64.xor
    local.get $r
    i64.xor
    i64.const 16
    i64.and
    i64.or
    local.get $r
    i64.eqz
    i64.extend_i32_u
    i64.const 6
    i64.shl
    i64.or
    local.get $r
    local.get $sign
    i64.and
    i64.eqz
    i32.eqz
    i64.extend_i32_u
    i64.const 7
    i64.shl
    i64.or
    local.get $cf_of
    i64.or
  )
  (func $fn_0x1000 (;3;) (type 2) (param $state i64) (param $entry i32) (param $ret i64) (result i32)
    (local $rax i64) (local $rcx i64) (local $rdx i64) (local $rbx i64) (local $rsp i64) (local $rbp i64) (local $rsi i64) (local $rdi i64) (local $r8 i64) (local $r9 i64) (local $r10 i64) (local $r11 i64) (local $r12 i64) (local $r13 i64) (local $r14 i64) (local $r15 i64) (local $rip i64) (local $cc_a i64) (local $cc_b i64) (local $cc_r i64) (local $scratch i64) (local $t0 i64) (local $t1 i64) (local $cc_op i32) (local $status i32) (local $func i32)
    local.get $state
    i64.load align=1
    local.set $rax
    local.get $state
    i64.load offset=8 align=1
    local.set $rcx
    local.get $state
    i64.load offset=16 align=1
    local.set $rdx
    local.get $state
    i64.load offset=24 align=1
    local.set $rbx
    local.get $state
    i64.load offset=32 align=1
    local.set $rsp
    local.get $state
    i64.load offset=40 align=1
    local.set $rbp
    local.get $state
    i64.load offset=48 align=1
    local.set $rsi
    local.get $state
    i64.load offset=56 align=1
    local.set $rdi
    local.get $state
    i64.load offset=64 align=1
    local.set $r8
    local.get $state
    i64.load offset=72 align=1
    local.set $r9
    local.get $state
    i64.load offset=80 align=1
    local.set $r10
    local.get $state
    i64.load offset=88 align=1
    local.set $r11
    local.get $state
    i64.load offset=96 align=1
    local.set $r12
    local.get $state
    i64.load offset=104 align=1
    local.set $r13
    local.get $state
    i64.load offset=112 align=1
    local.set $r14
    local.get $state
    i64.load offset=120 align=1
    local.set $r15
    local.get $state
    i32.load offset=148 align=1
    local.set $cc_op
    local.get $state
    i64.load offset=152 align=1
    local.set $cc_a
    local.get $state
    i64.load offset=160 align=1
    local.set $cc_b
    local.get $state
    i64.load offset=168 align=1
    local.set $cc_r
    block ;; label = @1
      loop ;; label = @2
        block ;; label = @3
          block ;; label = @4
            block ;; label = @5
              block ;; label = @6
                local.get $entry
                br_table 1 (;@5;) 2 (;@4;) 3 (;@3;) 0 (;@6;)
              end
              unreachable
            end
            local.get $rsp
            i64.const 8
            i64.sub
            local.set $t0
            local.get $t0
            i64.const 4101
            i64.store align=1
            local.get $t0
            local.set $rsp
            i64.const 4107
            local.set $rip
            global.get $depth
            i32.const 500
            i32.ge_u
            if ;; label = @5
              i32.const 1
              local.set $status
              br 4 (;@1;)
            end
            local.get $state
            local.get $rax
            i64.store align=1
            local.get $state
            local.get $rcx
            i64.store offset=8 align=1
            local.get $state
            local.get $rdx
            i64.store offset=16 align=1
            local.get $state
            local.get $rbx
            i64.store offset=24 align=1
            local.get $state
            local.get $rsp
            i64.store offset=32 align=1
            local.get $state
            local.get $rbp
            i64.store offset=40 align=1
            local.get $state
            local.get $rsi
            i64.store offset=48 align=1
            local.get $state
            local.get $rdi
            i64.store offset=56 align=1
            local.get $state
            local.get $r8
            i64.store offset=64 align=1
            local.get $state
            local.get $r9
            i64.store offset=72 align=1
            local.get $state
            local.get $r10
            i64.store offset=80 align=1
            local.get $state
            local.get $r11
            i64.store offset=88 align=1
            local.get $state
            local.get $r12
            i64.store offset=96 align=1
            local.get $state
            local.get $r13
            i64.store offset=104 align=1
            local.get $state
            local.get $r14
            i64.store offset=112 align=1
            local.get $state
            local.get $r15
            i64.store offset=120 align=1
            local.get $state
            local.get $cc_op
            i32.store offset=148 align=1
            local.get $state
            local.get $cc_a
            i64.store offset=152 align=1
            local.get $state
            local.get $cc_b
            i64.store offset=160 align=1
            local.get $state
            local.get $cc_r
            i64.store offset=168 align=1
            global.get $depth
            i32.const 1
            i32.add
            global.set $depth
            local.get $state
            i32.const 0
            i64.const 4101
            call $fn_0x100b
            global.get $depth
            i32.const -1
            i32.add
            global.set $depth
            local.tee $status
            if ;; label = @5
              local.get $status
              return
            end
            local.get $state
            i64.load align=1
            local.set $rax
            local.get $state
            i64.load offset=8 align=1
            local.set $rcx
            local.get $state
            i64.load offset=16 align=1
            local.set $rdx
            local.get $state
            i64.load offset=24 align=1
            local.set $rbx
            local.get $state
            i64.load offset=32 align=1
            local.set $rsp
            local.get $state
            i64.load offset=40 align=1
            local.set $rbp
            local.get $state
            i64.load offset=48 align=1
            local.set $rsi
            local.get $state
            i64.load offset=56 align=1
            local.set $rdi
            local.get $state
            i64.load offset=64 align=1
            local.set $r8
            local.get $state
            i64.load offset=72 align=1
            local.set $r9
            local.get $state
            i64.load offset=80 align=1
            local.set $r10
            local.get $state
            i64.load offset=88 align=1
            local.set $r11
            local.get $state
            i64.load offset=96 align=1
            local.set $r12
            local.get $state
            i64.load offset=104 align=1
            local.set $r13
            local.get $state
            i64.load offset=112 align=1
            local.set $r14
            local.get $state
            i64.load offset=120 align=1
            local.set $r15
            local.get $state
            i32.load offset=148 align=1
            local.set $cc_op
            local.get $state
            i64.load offset=152 align=1
            local.set $cc_a
            local.get $state
            i64.load offset=160 align=1
            local.set $cc_b
            local.get $state
            i64.load offset=168 align=1
            local.set $cc_r
          end
          local.get $rsp
          i64.const 8
          i64.sub
          local.set $t0
          local.get $t0
          i64.const 4106
          i64.store align=1
          local.get $t0
          local.set $rsp
          i64.const 4113
          local.set $rip
          global.get $depth
          i32.const 500
          i32.ge_u
          if ;; label = @4
            i32.const 1
            local.set $status
            br 3 (;@1;)
          end
          local.get $state
          local.get $rax
          i64.store align=1
          local.get $state
          local.get $rcx
          i64.store offset=8 align=1
          local.get $state
          local.get $rdx
          i64.store offset=16 align=1
          local.get $state
          local.get $rbx
          i64.store offset=24 align=1
          local.get $state
          local.get $rsp
          i64.store offset=32 align=1
          local.get $state
          local.get $rbp
          i64.store offset=40 align=1
          local.get $state
          local.get $rsi
          i64.store offset=48 align=1
          local.get $state
          local.get $rdi
          i64.store offset=56 align=1
          local.get $state
          local.get $r8
          i64.store offset=64 align=1
          local.get $state
          local.get $r9
          i64.store offset=72 align=1
          local.get $state
          local.get $r10
          i64.store offset=80 align=1
          local.get $state
          local.get $r11
          i64.store offset=88 align=1
          local.get $state
          local.get $r12
          i64.store offset=96 align=1
          local.get $state
          local.get $r13
          i64.store offset=104 align=1
          local.get $state
          local.get $r14
          i64.store offset=112 align=1
          local.get $state
          local.get $r15
          i64.store offset=120 align=1
          local.get $state
          local.get $cc_op
          i32.store offset=148 align=1
          local.get $state
          local.get $cc_a
          i64.store offset=152 align=1
          local.get $state
          local.get $cc_b
          i64.store offset=160 align=1
          local.get $state
          local.get $cc_r
          i64.store offset=168 align=1
          global.get $depth
          i32.const 1
          i32.add
          global.set $depth
          local.get $state
          i32.const 0
          i64.const 4106
          call $fn_0x1011
          global.get $depth
          i32.const -1
          i32.add
          global.set $depth
          local.tee $status
          if ;; label = @4
            local.get $status
            return
          end
          local.get $state
          i64.load align=1
          local.set $rax
          local.get $state
          i64.load offset=8 align=1
          local.set $rcx
          local.get $state
          i64.load offset=16 align=1
          local.set $rdx
          local.get $state
          i64.load offset=24 align=1
          local.set $rbx
          local.get $state
          i64.load offset=32 align=1
          local.set $rsp
          local.get $state
          i64.load offset=40 align=1
          local.set $rbp
          local.get $state
          i64.load offset=48 align=1
          local.set $rsi
          local.get $state
          i64.load offset=56 align=1
          local.set $rdi
          local.get $state
          i64.load offset=64 align=1
          local.set $r8
          local.get $state
          i64.load offset=72 align=1
          local.set $r9
          local.get $state
          i64.load offset=80 align=1
          local.set $r10
          local.get $state
          i64.load offset=88 align=1
          local.set $r11
          local.get $state
          i64.load offset=96 align=1
          local.set $r12
          local.get $state
          i64.load offset=104 align=1
          local.set $r13
          local.get $state
          i64.load offset=112 align=1
          local.set $r14
          local.get $state
          i64.load offset=120 align=1
          local.set $r15
          local.get $state
          i32.load offset=148 align=1
          local.set $cc_op
          local.get $state
          i64.load offset=152 align=1
          local.set $cc_a
          local.get $state
          i64.load offset=160 align=1
          local.set $cc_b
          local.get $state
          i64.load offset=168 align=1
          local.set $cc_r
        end
        local.get $rsp
        i64.load align=1
        local.set $t0
        local.get $rsp
        i64.const 8
        i64.add
        local.set $rsp
        local.get $t0
        local.tee $rip
        local.get $ret
        i64.ne
        local.set $status
        br 1 (;@1;)
      end
      unreachable
    end
    local.get $state
    local.get $rax
    i64.store align=1
    local.get $state
    local.get $rcx
    i64.store offset=8 align=1
    local.get $state
    local.get $rdx
    i64.store offset=16 align=1
    local.get $state
    local.get $rbx
    i64.store offset=24 align=1
    local.get $state
    local.get $rsp
    i64.store offset=32 align=1
    local.get $state
    local.get $rbp
    i64.store offset=40 align=1
    local.get $state
    local.get $rsi
    i64.store offset=48 align=1
    local.get $state
    local.get $rdi
    i64.store offset=56 align=1
    local.get $state
    local.get $r8
    i64.store offset=64 align=1
    local.get $state
    local.get $r9
    i64.store offset=72 align=1
    local.get $state
    local.get $r10
    i64.store offset=80 align=1
    local.get $state
    local.get $r11
    i64.store offset=88 align=1
    local.get $state
    local.get $r12
    i64.store offset=96 align=1
    local.get $state
    local.get $r13
    i64.store offset=104 align=1
    local.get $state
    local.get $r14
    i64.store offset=112 align=1
    local.get $state
    local.get $r15
    i64.store offset=120 align=1
    local.get $state
    local.get $cc_op
    i32.store offset=148 align=1
    local.get $state
    local.get $cc_a
    i64.store offset=152 align=1
    local.get $state
    local.get $cc_b
    i64.store offset=160 align=1
    local.get $state
    local.get $cc_r
    i64.store offset=168 align=1
    local.get $status
    if ;; label = @1
      local.get $state
      local.get $rip
      i64.store offset=128 align=1
    end
    local.get $status
  )
  (func $fn_0x100b (;4;) (type 2) (param $state i64) (param $entry i32) (param $ret i64) (result i32)
    (local $rax i64) (local $rcx i64) (local $rdx i64) (local $rbx i64) (local $rsp i64) (local $rbp i64) (local $rsi i64) (local $rdi i64) (local $r8 i64) (local $r9 i64) (local $r10 i64) (local $r11 i64) (local $r12 i64) (local $r13 i64) (local $r14 i64) (local $r15 i64) (local $rip i64) (local $cc_a i64) (local $cc_b i64) (local $cc_r i64) (local $scratch i64) (local $t0 i64) (local $cc_op i32) (local $status i32) (local $func i32)
    local.get $state
    i64.load align=1
    local.set $rax
    local.get $state
    i64.load offset=8 align=1
    local.set $rcx
    local.get $state
    i64.load offset=16 align=1
    local.set $rdx
    local.get $state
    i64.load offset=24 align=1
    local.set $rbx
    local.get $state
    i64.load offset=32 align=1
    local.set $rsp
    local.get $state
    i64.load offset=40 align=1
    local.set $rbp
    local.get $state
    i64.load offset=48 align=1
    local.set $rsi
    local.get $state
    i64.load offset=56 align=1
    local.set $rdi
    local.get $state
    i64.load offset=64 align=1
    local.set $r8
    local.get $state
    i64.load offset=72 align=1
    local.set $r9
    local.get $state
    i64.load offset=80 align=1
    local.set $r10
    local.get $state
    i64.load offset=88 align=1
    local.set $r11
    local.get $state
    i64.load offset=96 align=1
    local.set $r12
    local.get $state
    i64.load offset=104 align=1
    local.set $r13
    local.get $state
    i64.load offset=112 align=1
    local.set $r14
    local.get $state
    i64.load offset=120 align=1
    local.set $r15
    local.get $state
    i32.load offset=148 align=1
    local.set $cc_op
    local.get $state
    i64.load offset=152 align=1
    local.set $cc_a
    local.get $state
    i64.load offset=160 align=1
    local.set $cc_b
    local.get $state
    i64.load offset=168 align=1
    local.set $cc_r
    block ;; label = @1
      loop ;; label = @2
        block ;; label = @3
          block ;; label = @4
            local.get $entry
            br_table 1 (;@3;) 0 (;@4;)
          end
          unreachable
        end
        local.get $rax
        i64.const 1
        i64.add
        local.set $rax
        local.get $state
        local.get $rax
        i64.store align=1
        local.get $state
        local.get $rcx
        i64.store offset=8 align=1
        local.get $state
        local.get $rdx
        i64.store offset=16 align=1
        local.get $state
        local.get $rbx
        i64.store offset=24 align=1
        local.get $state
        local.get $rsp
        i64.store offset=32 align=1
        local.get $state
        local.get $rbp
        i64.store offset=40 align=1
        local.get $state
        local.get $rsi
        i64.store offset=48 align=1
        local.get $state
        local.get $rdi
        i64.store offset=56 align=1
        local.get $state
        local.get $r8
        i64.store offset=64 align=1
        local.get $state
        local.get $r9
        i64.store offset=72 align=1
        local.get $state
        local.get $r10
        i64.store offset=80 align=1
        local.get $state
        local.get $r11
        i64.store offset=88 align=1
        local.get $state
        local.get $r12
        i64.store offset=96 align=1
        local.get $state
        local.get $r13
        i64.store offset=104 align=1
        local.get $state
        local.get $r14
        i64.store offset=112 align=1
        local.get $state
        local.get $r15
        i64.store offset=120 align=1
        local.get $state
        local.get $cc_op
        i32.store offset=148 align=1
        local.get $state
        local.get $cc_a
        i64.store offset=152 align=1
        local.get $state
        local.get $cc_b
        i64.store offset=160 align=1
        local.get $state
        local.get $cc_r
        i64.store offset=168 align=1
        local.get $state
        i32.const 0
        local.get $ret
        return_call $fn_0x1011
      end
      unreachable
    end
    unreachable
  )
  (func $fn_0x1011 (;5;) (type 2) (param $state i64) (param $entry i32) (param $ret i64) (result i32)
    (local $rax i64) (local $rcx i64) (local $rdx i64) (local $rbx i64) (local $rsp i64) (local $rbp i64) (local $rsi i64) (local $rdi i64) (local $r8 i64) (local $r9 i64) (local $r10 i64) (local $r11 i64) (local $r12 i64) (local $r13 i64) (local $r14 i64) (local $r15 i64) (local $rip i64) (local $cc_a i64) (local $cc_b i64) (local $cc_r i64) (local $scratch i64) (local $t0 i64) (local $t1 i64) (local $cc_op i32) (local $status i32) (local $func i32)
    local.get $state
    i64.load align=1
    local.set $rax
    local.get $state
    i64.load offset=8 align=1
    local.set $rcx
    local.get $state
    i64.load offset=16 align=1
    local.set $rdx
    local.get $state
    i64.load offset=24 align=1
    local.set $rbx
    local.get $state
    i64.load offset=32 align=1
    local.set $rsp
    local.get $state
    i64.load offset=40 align=1
    local.set $rbp
    local.get $state
    i64.load offset=48 align=1
    local.set $rsi
    local.get $state
    i64.load offset=56 align=1
    local.set $rdi
    local.get $state
    i64.load offset=64 align=1
    local.set $r8
    local.get $state
    i64.load offset=72 align=1
    local.set $r9
    local.get $state
    i64.load offset=80 align=1
    local.set $r10
    local.get $state
    i64.load offset=88 align=1
    local.set $r11
    local.get $state
    i64.load offset=96 align=1
    local.set $r12
    local.get $state
    i64.load offset=104 align=1
    local.set $r13
    local.get $state
    i64.load offset=112 align=1
    local.set $r14
    local.get $state
    i64.load offset=120 align=1
    local.set $r15
    local.get $state
    i32.load offset=148 align=1
    local.set $cc_op
    local.get $state
    i64.load offset=152 align=1
    local.set $cc_a
    local.get $state
    i64.load offset=160 align=1
    local.set $cc_b
    local.get $state
    i64.load offset=168 align=1
    local.set $cc_r
    block ;; label = @1
      loop ;; label = @2
        block ;; label = @3
          block ;; label = @4
            local.get $entry
            br_table 1 (;@3;) 0 (;@4;)
          end
          unreachable
        end
        local.get $rax
        i64.const 2
        i64.add
        local.set $t0
        i32.const 4
        local.set $cc_op
        local.get $rax
        local.set $cc_a
        i64.const 2
        local.set $cc_b
        local.get $t0
        local.set $cc_r
        local.get $t0
        local.set $rax
        local.get $rsp
        i64.load align=1
        local.set $t0
        local.get $rsp
        i64.const 8
        i64.add
        local.set $rsp
        local.get $t0
        local.tee $rip
        local.get $ret
        i64.ne
        local.set $status
        br 1 (;@1;)
      end
      unreachable
    end
    local.get $state
    local.get $rax
    i64.store align=1
    local.get $state
    local.get $rcx
    i64.store offset=8 align=1
    local.get $state
    local.get $rdx
    i64.store offset=16 align=1
    local.get $state
    local.get $rbx
    i64.store offset=24 align=1
    local.get $state
    local.get $rsp
    i64.store offset=32 align=1
    local.get $state
    local.get $rbp
    i64.store offset=40 align=1
    local.get $state
    local.get $rsi
    i64.store offset=48 align=1
    local.get $state
    local.get $rdi
    i64.store offset=56 align=1
    local.get $state
    local.get $r8
    i64.store offset=64 align=1
    local.get $state
    local.get $r9
    i64.store offset=72 align=1
    local.get $state
    local.get $r10
    i64.store offset=80 align=1
    local.get $state
    local.get $r11
    i64.store offset=88 align=1
    local.get $state
    local.get $r12
    i64.store offset=96 align=1
    local.get $state
    local.get $r13
    i64.store offset=104 align=1
    local.get $state
    local.get $r14
    i64.store offset=112 align=1
    local.get $state
    local.get $r15
    i64.store offset=120 align=1
    local.get $state
    local.get $cc_op
    i32.store offset=148 align=1
    local.get $state
    local.get $cc_a
    i64.store offset=152 align=1
    local.get $state
    local.get $cc_b
    i64.store offset=160 align=1
    local.get $state
    local.get $cc_r
    i64.store offset=168 align=1
    local.get $status
    if ;; label = @1
      local.get $state
      local.get $rip
      i64.store offset=128 align=1
    end
    local.get $status
  )
)

