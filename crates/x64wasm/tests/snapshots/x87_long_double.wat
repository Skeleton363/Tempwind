(module
  (type (;0;) (func (param i64) (result i32)))
  (type (;1;) (func (param i32 i64 i64 i64) (result i64)))
  (type (;2;) (func (param i64 i32 i64) (result i32)))
  (type (;3;) (func (param i64) (result i32 i32)))
  (type (;4;) (func (param i64 i32 i64 i32 i32 i32) (result i64)))
  (type (;5;) (func (param i64 i32 i32) (result i64)))
  (type (;6;) (func (param i64 i32 i64 i32 i32 i32 i32) (result i32)))
  (type (;7;) (func (result i32)))
  (import "env" "memory" (memory (;0;) i64 1))
  (import "env" "table" (table $table (;0;) 0 funcref))
  (import "env" "table_base" (global $table_base (;0;) i32))
  (import "env" "lookup" (global $lookup (;1;) i64))
  (import "env" "depth" (global $depth (;2;) (mut i32)))
  (import "fp" "f80_add" (func $fp.f80_add (;0;) (type 4) (param i64 i32 i64 i32 i32 i32) (result i64)))
  (import "fp" "f80_load" (func $fp.f80_load (;1;) (type 5) (param i64 i32 i32) (result i64)))
  (import "fp" "f80_cmp" (func $fp.f80_cmp (;2;) (type 6) (param i64 i32 i64 i32 i32 i32 i32) (result i32)))
  (import "fp" "status" (func $fp.status (;3;) (type 7) (result i32)))
  (export "run" (func $run))
  (elem (;0;) (table $table) (global.get $table_base) func $fn_0x1000)
  (func $run (;4;) (type 0) (param $state i64) (result i32)
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
  (func $lookup (;5;) (type 3) (param $rip i64) (result i32 i32)
    (local $slots i64) (local $mask i64) (local $i i64) (local $at i64) (local $key i64)
    global.get $lookup
    i64.load align=1
    local.set $slots
    global.get $lookup
    i64.load offset=8 align=1
    local.tee $mask
    local.get $rip
    i64.const -7046029254386353131
    i64.mul
    i64.const 32
    i64.shr_u
    i64.and
    local.set $i
    loop ;; label = @1
      local.get $slots
      local.get $i
      i64.const 4
      i64.shl
      i64.add
      local.tee $at
      i64.load align=1
      local.tee $key
      i64.eqz
      if ;; label = @2
        i32.const -1
        i32.const 0
        return
      end
      local.get $key
      local.get $rip
      i64.eq
      if ;; label = @2
        local.get $at
        i32.load offset=8 align=1
        local.get $at
        i32.load offset=12 align=1
        return
      end
      local.get $i
      i64.const 1
      i64.add
      local.get $mask
      i64.and
      local.set $i
      br 0 (;@1;)
    end
    unreachable
  )
  (func $flags (;6;) (type 1) (param $op i32) (param $a i64) (param $b i64) (param $r i64) (result i64)
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
    block ;; label = @1
      block ;; label = @2
        block ;; label = @3
          block ;; label = @4
            block ;; label = @5
              block ;; label = @6
                block ;; label = @7
                  block ;; label = @8
                    block ;; label = @9
                      block ;; label = @10
                        block ;; label = @11
                          block ;; label = @12
                            local.get $op
                            i32.const 1
                            i32.sub
                            i32.const 2
                            i32.shr_u
                            br_table 0 (;@12;) 1 (;@11;) 2 (;@10;) 3 (;@9;) 4 (;@8;) 5 (;@7;) 6 (;@6;) 7 (;@5;) 8 (;@4;) 9 (;@3;) 10 (;@2;) 10 (;@2;)
                          end
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
                          i64.const 0
                          i64.ne
                          i64.extend_i32_u
                          i64.const 11
                          i64.shl
                          i64.or
                          local.set $cf_of
                          br 10 (;@1;)
                        end
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
                        i64.const 0
                        i64.ne
                        i64.extend_i32_u
                        i64.const 11
                        i64.shl
                        i64.or
                        local.set $cf_of
                        br 9 (;@1;)
                      end
                      i32.const 0
                      i64.extend_i32_u
                      i32.const 0
                      i64.extend_i32_u
                      i64.const 11
                      i64.shl
                      i64.or
                      local.set $cf_of
                      br 8 (;@1;)
                    end
                    local.get $a
                    local.get $b
                    i64.and
                    local.get $a
                    local.get $b
                    i64.xor
                    local.get $r
                    i64.const -1
                    i64.xor
                    i64.and
                    i64.or
                    local.get $sign
                    i64.and
                    i64.const 0
                    i64.ne
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
                    i64.const 0
                    i64.ne
                    i64.extend_i32_u
                    i64.const 11
                    i64.shl
                    i64.or
                    local.set $cf_of
                    br 7 (;@1;)
                  end
                  local.get $a
                  i64.const -1
                  i64.xor
                  local.get $b
                  i64.and
                  local.get $a
                  local.get $b
                  i64.xor
                  i64.const -1
                  i64.xor
                  local.get $r
                  i64.and
                  i64.or
                  local.get $sign
                  i64.and
                  i64.const 0
                  i64.ne
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
                  i64.const 0
                  i64.ne
                  i64.extend_i32_u
                  i64.const 11
                  i64.shl
                  i64.or
                  local.set $cf_of
                  br 6 (;@1;)
                end
                local.get $b
                i64.eqz
                i32.eqz
                i64.extend_i32_u
                local.get $r
                local.get $sign
                i64.eq
                i64.extend_i32_u
                i64.const 11
                i64.shl
                i64.or
                local.set $cf_of
                br 5 (;@1;)
              end
              local.get $b
              i64.eqz
              i32.eqz
              i64.extend_i32_u
              local.get $a
              local.get $sign
              i64.eq
              i64.extend_i32_u
              i64.const 11
              i64.shl
              i64.or
              local.set $cf_of
              br 4 (;@1;)
            end
            local.get $a
            local.get $b
            i64.const 1
            i64.sub
            i64.shl
            local.get $sign
            i64.and
            i64.const 0
            i64.ne
            i64.extend_i32_u
            local.get $a
            local.get $r
            i64.xor
            local.get $sign
            i64.and
            i64.const 0
            i64.ne
            i64.extend_i32_u
            i64.const 11
            i64.shl
            i64.or
            local.set $cf_of
            br 3 (;@1;)
          end
          local.get $a
          local.get $b
          i64.const 1
          i64.sub
          i64.shr_u
          i32.wrap_i64
          i32.const 1
          i32.and
          i64.extend_i32_u
          local.get $a
          local.get $r
          i64.xor
          local.get $sign
          i64.and
          i64.const 0
          i64.ne
          i64.extend_i32_u
          i64.const 11
          i64.shl
          i64.or
          local.set $cf_of
          br 2 (;@1;)
        end
        local.get $a
        local.get $sign
        i64.xor
        local.get $sign
        i64.sub
        local.get $b
        i64.const 1
        i64.sub
        i64.shr_s
        i32.wrap_i64
        i32.const 1
        i32.and
        i64.extend_i32_u
        local.get $a
        local.get $r
        i64.xor
        local.get $sign
        i64.and
        i64.const 0
        i64.ne
        i64.extend_i32_u
        i64.const 11
        i64.shl
        i64.or
        local.set $cf_of
        br 1 (;@1;)
      end
      local.get $a
      i64.eqz
      i32.eqz
      i64.extend_i32_u
      local.get $a
      i64.eqz
      i32.eqz
      i64.extend_i32_u
      i64.const 11
      i64.shl
      i64.or
      local.set $cf_of
      br 0 (;@1;)
    end
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
  (func $fn_0x1000 (;7;) (type 2) (param $state i64) (param $entry i32) (param $ret i64) (result i32)
    (local $rax i64) (local $rcx i64) (local $rdx i64) (local $rbx i64) (local $rsp i64) (local $rbp i64) (local $rsi i64) (local $rdi i64) (local $r8 i64) (local $r9 i64) (local $r10 i64) (local $r11 i64) (local $r12 i64) (local $r13 i64) (local $r14 i64) (local $r15 i64) (local $rip i64) (local $cc_a i64) (local $cc_b i64) (local $cc_r i64) (local $scratch i64) (local $t0 i64) (local $t1 i64) (local $t2 i64) (local $cc_op i32) (local $status i32) (local $func i32) (local $x87_top i32) (local $x87_fsw i32) (local $x87_ftw i32) (local $x87_fcw i32) (local $x87_st i32) (local $x87_p i32) (local $x87_q i32) (local $x87_ase i32) (local $x87_bse i32) (local $x87_rse i32) (local $x87_am i64) (local $x87_bm i64) (local $x87_rm i64) (local $x87_addr i64)
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
        local.get $rcx
        local.set $x87_addr
        local.get $state
        i32.load16_u offset=518 align=1
        local.tee $x87_fsw
        i32.const 128
        i32.and
        if ;; label = @3
          local.get $state
          i32.const 2
          i32.store offset=144 align=1
          i64.const 4096
          local.set $rip
          i32.const 2
          local.set $status
          br 2 (;@1;)
        end
        local.get $x87_fsw
        i32.const 11
        i32.shr_u
        i32.const 7
        i32.and
        local.set $x87_top
        local.get $state
        i32.load16_u offset=520 align=1
        local.set $x87_ftw
        local.get $state
        i32.load16_u offset=516 align=1
        local.set $x87_fcw
        local.get $x87_addr
        i64.load align=1
        local.set $x87_bm
        local.get $x87_addr
        i32.load16_u offset=8 align=1
        local.set $x87_bse
        local.get $x87_top
        i32.const -1
        i32.add
        i32.const 7
        i32.and
        local.set $x87_p
        local.get $x87_ftw
        local.get $x87_p
        i32.const 1
        i32.shl
        i32.shr_u
        i32.const 3
        i32.and
        i32.const 3
        i32.eq
        i32.eqz
        local.tee $x87_rse
        if ;; label = @3
          local.get $x87_rse
          local.set $x87_ase
          local.get $x87_fcw
          i32.const 1
          i32.and
          i32.eqz
          if ;; label = @4
            local.get $state
            i32.const 2
            i32.store offset=144 align=1
            i64.const 4096
            local.set $rip
            i32.const 2
            local.set $status
            br 3 (;@1;)
          end
          i64.const -4611686018427387904
          local.set $x87_rm
          i32.const 65535
          local.set $x87_rse
          i32.const 65
          local.set $x87_st
          local.get $x87_st
          local.get $x87_ase
          i32.const 9
          i32.shl
          i32.or
          local.set $x87_st
        else
          local.get $x87_bm
          local.set $x87_rm
          local.get $x87_bse
          local.set $x87_rse
          i32.const 0
          local.set $x87_st
        end
        local.get $state
        local.get $x87_p
        i64.extend_i32_u
        i64.const 4
        i64.shl
        i64.add
        local.get $x87_rm
        i64.store offset=528 align=1
        local.get $state
        local.get $x87_p
        i64.extend_i32_u
        i64.const 4
        i64.shl
        i64.add
        local.get $x87_rse
        i32.store16 offset=536 align=1
        local.get $x87_ftw
        i32.const 3
        local.get $x87_p
        i32.const 1
        i32.shl
        i32.shl
        i32.const -1
        i32.xor
        i32.and
        i32.const 2
        i32.const 2
        local.get $x87_rm
        i64.eqz
        i32.sub
        local.get $x87_rm
        i64.const 0
        i64.ge_s
        i32.const 1
        i32.shl
        local.get $x87_rse
        i32.const 32767
        i32.and
        i32.eqz
        select
        local.get $x87_rse
        i32.const 32767
        i32.and
        i32.const 32767
        i32.eq
        select
        local.get $x87_p
        i32.const 1
        i32.shl
        i32.shl
        i32.or
        local.set $x87_ftw
        local.get $x87_p
        local.set $x87_top
        local.get $state
        local.get $x87_fsw
        i32.const -14849
        i32.and
        local.get $x87_st
        i32.const 639
        i32.and
        i32.or
        local.get $x87_top
        i32.const 11
        i32.shl
        i32.or
        i32.store16 offset=518 align=1
        local.get $state
        local.get $x87_ftw
        i32.store16 offset=520 align=1
        local.get $rdx
        local.set $x87_addr
        local.get $state
        i32.load16_u offset=518 align=1
        local.tee $x87_fsw
        i32.const 128
        i32.and
        if ;; label = @3
          local.get $state
          i32.const 2
          i32.store offset=144 align=1
          i64.const 4098
          local.set $rip
          i32.const 2
          local.set $status
          br 2 (;@1;)
        end
        local.get $x87_fsw
        i32.const 11
        i32.shr_u
        i32.const 7
        i32.and
        local.set $x87_top
        local.get $state
        i32.load16_u offset=520 align=1
        local.set $x87_ftw
        local.get $state
        i32.load16_u offset=516 align=1
        local.set $x87_fcw
        local.get $x87_addr
        i64.load32_s align=1
        local.set $x87_bm
        local.get $x87_top
        i32.const -1
        i32.add
        i32.const 7
        i32.and
        local.set $x87_p
        local.get $x87_ftw
        local.get $x87_p
        i32.const 1
        i32.shl
        i32.shr_u
        i32.const 3
        i32.and
        i32.const 3
        i32.eq
        i32.eqz
        local.tee $x87_rse
        if ;; label = @3
          local.get $x87_rse
          local.set $x87_ase
          local.get $x87_fcw
          i32.const 1
          i32.and
          i32.eqz
          if ;; label = @4
            local.get $state
            i32.const 2
            i32.store offset=144 align=1
            i64.const 4098
            local.set $rip
            i32.const 2
            local.set $status
            br 3 (;@1;)
          end
          i64.const -4611686018427387904
          local.set $x87_rm
          i32.const 65535
          local.set $x87_rse
          i32.const 65
          local.set $x87_st
          local.get $x87_st
          local.get $x87_ase
          i32.const 9
          i32.shl
          i32.or
          local.set $x87_st
        else
          local.get $x87_bm
          i32.const 4
          local.get $x87_fcw
          call $fp.f80_load
          local.set $x87_rm
          call $fp.status
          local.tee $x87_st
          i32.const 128
          i32.and
          if ;; label = @4
            local.get $state
            i32.const 2
            i32.store offset=144 align=1
            i64.const 4098
            local.set $rip
            i32.const 2
            local.set $status
            br 3 (;@1;)
          end
          local.get $x87_st
          i32.const 16
          i32.shr_u
          local.set $x87_rse
        end
        local.get $state
        local.get $x87_p
        i64.extend_i32_u
        i64.const 4
        i64.shl
        i64.add
        local.get $x87_rm
        i64.store offset=528 align=1
        local.get $state
        local.get $x87_p
        i64.extend_i32_u
        i64.const 4
        i64.shl
        i64.add
        local.get $x87_rse
        i32.store16 offset=536 align=1
        local.get $x87_ftw
        i32.const 3
        local.get $x87_p
        i32.const 1
        i32.shl
        i32.shl
        i32.const -1
        i32.xor
        i32.and
        i32.const 2
        i32.const 2
        local.get $x87_rm
        i64.eqz
        i32.sub
        local.get $x87_rm
        i64.const 0
        i64.ge_s
        i32.const 1
        i32.shl
        local.get $x87_rse
        i32.const 32767
        i32.and
        i32.eqz
        select
        local.get $x87_rse
        i32.const 32767
        i32.and
        i32.const 32767
        i32.eq
        select
        local.get $x87_p
        i32.const 1
        i32.shl
        i32.shl
        i32.or
        local.set $x87_ftw
        local.get $x87_p
        local.set $x87_top
        local.get $state
        local.get $x87_fsw
        i32.const -14849
        i32.and
        local.get $x87_st
        i32.const 639
        i32.and
        i32.or
        local.get $x87_top
        i32.const 11
        i32.shl
        i32.or
        i32.store16 offset=518 align=1
        local.get $state
        local.get $x87_ftw
        i32.store16 offset=520 align=1
        local.get $state
        i32.load16_u offset=518 align=1
        local.tee $x87_fsw
        i32.const 128
        i32.and
        if ;; label = @3
          local.get $state
          i32.const 2
          i32.store offset=144 align=1
          i64.const 4100
          local.set $rip
          i32.const 2
          local.set $status
          br 2 (;@1;)
        end
        local.get $x87_fsw
        i32.const 11
        i32.shr_u
        i32.const 7
        i32.and
        local.set $x87_top
        local.get $state
        i32.load16_u offset=520 align=1
        local.set $x87_ftw
        local.get $state
        i32.load16_u offset=516 align=1
        local.set $x87_fcw
        local.get $x87_top
        i32.const 1
        i32.add
        i32.const 7
        i32.and
        local.set $x87_p
        local.get $x87_top
        local.set $x87_q
        local.get $state
        local.get $x87_q
        i64.extend_i32_u
        i64.const 4
        i64.shl
        i64.add
        i64.load offset=528 align=1
        local.set $x87_bm
        local.get $state
        local.get $x87_q
        i64.extend_i32_u
        i64.const 4
        i64.shl
        i64.add
        i32.load16_u offset=536 align=1
        local.set $x87_bse
        local.get $x87_ftw
        local.get $x87_p
        i32.const 1
        i32.shl
        i32.shr_u
        i32.const 3
        i32.and
        i32.const 3
        i32.eq
        local.get $x87_ftw
        local.get $x87_q
        i32.const 1
        i32.shl
        i32.shr_u
        i32.const 3
        i32.and
        i32.const 3
        i32.eq
        i32.or
        if ;; label = @3
          local.get $x87_fcw
          i32.const 1
          i32.and
          i32.eqz
          if ;; label = @4
            local.get $state
            i32.const 2
            i32.store offset=144 align=1
            i64.const 4100
            local.set $rip
            i32.const 2
            local.set $status
            br 3 (;@1;)
          end
          i64.const -4611686018427387904
          local.set $x87_rm
          i32.const 65535
          local.set $x87_rse
          i32.const 65
          local.set $x87_st
        else
          local.get $state
          local.get $x87_p
          i64.extend_i32_u
          i64.const 4
          i64.shl
          i64.add
          i64.load offset=528 align=1
          local.set $x87_am
          local.get $state
          local.get $x87_p
          i64.extend_i32_u
          i64.const 4
          i64.shl
          i64.add
          i32.load16_u offset=536 align=1
          local.set $x87_ase
          local.get $x87_am
          local.get $x87_ase
          local.get $x87_bm
          local.get $x87_bse
          i32.const 0
          local.get $x87_fcw
          call $fp.f80_add
          local.set $x87_rm
          call $fp.status
          local.tee $x87_st
          i32.const 128
          i32.and
          if ;; label = @4
            local.get $state
            i32.const 2
            i32.store offset=144 align=1
            i64.const 4100
            local.set $rip
            i32.const 2
            local.set $status
            br 3 (;@1;)
          end
          local.get $x87_st
          i32.const 16
          i32.shr_u
          local.set $x87_rse
        end
        local.get $state
        local.get $x87_p
        i64.extend_i32_u
        i64.const 4
        i64.shl
        i64.add
        local.get $x87_rm
        i64.store offset=528 align=1
        local.get $state
        local.get $x87_p
        i64.extend_i32_u
        i64.const 4
        i64.shl
        i64.add
        local.get $x87_rse
        i32.store16 offset=536 align=1
        local.get $x87_ftw
        i32.const 3
        local.get $x87_p
        i32.const 1
        i32.shl
        i32.shl
        i32.const -1
        i32.xor
        i32.and
        i32.const 2
        i32.const 2
        local.get $x87_rm
        i64.eqz
        i32.sub
        local.get $x87_rm
        i64.const 0
        i64.ge_s
        i32.const 1
        i32.shl
        local.get $x87_rse
        i32.const 32767
        i32.and
        i32.eqz
        select
        local.get $x87_rse
        i32.const 32767
        i32.and
        i32.const 32767
        i32.eq
        select
        local.get $x87_p
        i32.const 1
        i32.shl
        i32.shl
        i32.or
        local.set $x87_ftw
        local.get $x87_ftw
        i32.const 3
        local.get $x87_top
        i32.const 1
        i32.shl
        i32.shl
        i32.const -1
        i32.xor
        i32.and
        i32.const 3
        local.get $x87_top
        i32.const 1
        i32.shl
        i32.shl
        i32.or
        local.set $x87_ftw
        local.get $x87_top
        i32.const 1
        i32.add
        i32.const 7
        i32.and
        local.set $x87_top
        local.get $state
        local.get $x87_fsw
        i32.const -14849
        i32.and
        local.get $x87_st
        i32.const 639
        i32.and
        i32.or
        local.get $x87_top
        i32.const 11
        i32.shl
        i32.or
        i32.store16 offset=518 align=1
        local.get $state
        local.get $x87_ftw
        i32.store16 offset=520 align=1
        local.get $state
        i32.load16_u offset=518 align=1
        local.tee $x87_fsw
        i32.const 128
        i32.and
        if ;; label = @3
          local.get $state
          i32.const 2
          i32.store offset=144 align=1
          i64.const 4102
          local.set $rip
          i32.const 2
          local.set $status
          br 2 (;@1;)
        end
        local.get $x87_fsw
        i32.const 11
        i32.shr_u
        i32.const 7
        i32.and
        local.set $x87_top
        local.get $state
        i32.load16_u offset=520 align=1
        local.set $x87_ftw
        local.get $state
        i32.load16_u offset=516 align=1
        local.set $x87_fcw
        local.get $x87_top
        i32.const -1
        i32.add
        i32.const 7
        i32.and
        local.set $x87_p
        local.get $x87_ftw
        local.get $x87_p
        i32.const 1
        i32.shl
        i32.shr_u
        i32.const 3
        i32.and
        i32.const 3
        i32.eq
        i32.eqz
        local.tee $x87_rse
        if ;; label = @3
          local.get $x87_rse
          local.set $x87_ase
          local.get $x87_fcw
          i32.const 1
          i32.and
          i32.eqz
          if ;; label = @4
            local.get $state
            i32.const 2
            i32.store offset=144 align=1
            i64.const 4102
            local.set $rip
            i32.const 2
            local.set $status
            br 3 (;@1;)
          end
          i64.const -4611686018427387904
          local.set $x87_rm
          i32.const 65535
          local.set $x87_rse
          i32.const 65
          local.set $x87_st
          local.get $x87_st
          local.get $x87_ase
          i32.const 9
          i32.shl
          i32.or
          local.set $x87_st
        else
          i64.const 0
          local.set $x87_rm
          i32.const 0
          local.set $x87_rse
          i32.const 0
          local.set $x87_st
        end
        local.get $state
        local.get $x87_p
        i64.extend_i32_u
        i64.const 4
        i64.shl
        i64.add
        local.get $x87_rm
        i64.store offset=528 align=1
        local.get $state
        local.get $x87_p
        i64.extend_i32_u
        i64.const 4
        i64.shl
        i64.add
        local.get $x87_rse
        i32.store16 offset=536 align=1
        local.get $x87_ftw
        i32.const 3
        local.get $x87_p
        i32.const 1
        i32.shl
        i32.shl
        i32.const -1
        i32.xor
        i32.and
        i32.const 2
        i32.const 2
        local.get $x87_rm
        i64.eqz
        i32.sub
        local.get $x87_rm
        i64.const 0
        i64.ge_s
        i32.const 1
        i32.shl
        local.get $x87_rse
        i32.const 32767
        i32.and
        i32.eqz
        select
        local.get $x87_rse
        i32.const 32767
        i32.and
        i32.const 32767
        i32.eq
        select
        local.get $x87_p
        i32.const 1
        i32.shl
        i32.shl
        i32.or
        local.set $x87_ftw
        local.get $x87_p
        local.set $x87_top
        local.get $state
        local.get $x87_fsw
        i32.const -14849
        i32.and
        local.get $x87_st
        i32.const 639
        i32.and
        i32.or
        local.get $x87_top
        i32.const 11
        i32.shl
        i32.or
        i32.store16 offset=518 align=1
        local.get $state
        local.get $x87_ftw
        i32.store16 offset=520 align=1
        local.get $state
        i32.load16_u offset=518 align=1
        local.tee $x87_fsw
        i32.const 128
        i32.and
        if ;; label = @3
          local.get $state
          i32.const 2
          i32.store offset=144 align=1
          i64.const 4104
          local.set $rip
          i32.const 2
          local.set $status
          br 2 (;@1;)
        end
        local.get $x87_fsw
        i32.const 11
        i32.shr_u
        i32.const 7
        i32.and
        local.set $x87_top
        local.get $state
        i32.load16_u offset=520 align=1
        local.set $x87_ftw
        local.get $state
        i32.load16_u offset=516 align=1
        local.set $x87_fcw
        local.get $x87_top
        i32.const 1
        i32.add
        i32.const 7
        i32.and
        local.set $x87_q
        local.get $state
        local.get $x87_q
        i64.extend_i32_u
        i64.const 4
        i64.shl
        i64.add
        i64.load offset=528 align=1
        local.set $x87_bm
        local.get $state
        local.get $x87_q
        i64.extend_i32_u
        i64.const 4
        i64.shl
        i64.add
        i32.load16_u offset=536 align=1
        local.set $x87_bse
        local.get $x87_ftw
        local.get $x87_top
        i32.const 1
        i32.shl
        i32.shr_u
        i32.const 3
        i32.and
        i32.const 3
        i32.eq
        local.get $x87_ftw
        local.get $x87_q
        i32.const 1
        i32.shl
        i32.shr_u
        i32.const 3
        i32.and
        i32.const 3
        i32.eq
        i32.or
        if ;; label = @3
          local.get $x87_fcw
          i32.const 1
          i32.and
          i32.eqz
          if ;; label = @4
            local.get $state
            i32.const 2
            i32.store offset=144 align=1
            i64.const 4104
            local.set $rip
            i32.const 2
            local.set $status
            br 3 (;@1;)
          end
          i64.const -4611686018427387904
          local.set $x87_rm
          i32.const 65535
          local.set $x87_rse
          i32.const 17729
          local.set $x87_st
        else
          local.get $state
          local.get $x87_top
          i64.extend_i32_u
          i64.const 4
          i64.shl
          i64.add
          i64.load offset=528 align=1
          local.set $x87_am
          local.get $state
          local.get $x87_top
          i64.extend_i32_u
          i64.const 4
          i64.shl
          i64.add
          i32.load16_u offset=536 align=1
          local.set $x87_ase
          local.get $x87_am
          local.get $x87_ase
          local.get $x87_bm
          local.get $x87_bse
          i32.const 0
          i32.const 1
          local.get $x87_fcw
          call $fp.f80_cmp
          drop
          call $fp.status
          local.tee $x87_st
          i32.const 128
          i32.and
          if ;; label = @4
            local.get $state
            i32.const 2
            i32.store offset=144 align=1
            i64.const 4104
            local.set $rip
            i32.const 2
            local.set $status
            br 3 (;@1;)
          end
          local.get $x87_st
          local.get $x87_fsw
          i32.const 512
          i32.and
          i32.or
          local.set $x87_st
        end
        local.get $x87_ftw
        i32.const 3
        local.get $x87_top
        i32.const 1
        i32.shl
        i32.shl
        i32.const -1
        i32.xor
        i32.and
        i32.const 3
        local.get $x87_top
        i32.const 1
        i32.shl
        i32.shl
        i32.or
        local.set $x87_ftw
        local.get $x87_top
        i32.const 1
        i32.add
        i32.const 7
        i32.and
        local.set $x87_top
        local.get $state
        local.get $x87_fsw
        i32.const -14849
        i32.and
        local.get $x87_st
        i32.const 639
        i32.and
        i32.or
        local.get $x87_top
        i32.const 11
        i32.shl
        i32.or
        i32.store16 offset=518 align=1
        local.get $state
        local.get $x87_ftw
        i32.store16 offset=520 align=1
        local.get $x87_st
        i32.const 8
        i32.shr_u
        i32.const 69
        i32.and
        i64.extend_i32_u
        local.set $t0
        local.get $t0
        local.set $cc_a
        i32.const 0
        local.set $cc_op
        local.get $r8
        local.set $x87_addr
        local.get $state
        i32.load16_u offset=518 align=1
        local.tee $x87_fsw
        i32.const 128
        i32.and
        if ;; label = @3
          local.get $state
          i32.const 2
          i32.store offset=144 align=1
          i64.const 4106
          local.set $rip
          i32.const 2
          local.set $status
          br 2 (;@1;)
        end
        local.get $x87_fsw
        i32.const 11
        i32.shr_u
        i32.const 7
        i32.and
        local.set $x87_top
        local.get $state
        i32.load16_u offset=520 align=1
        local.set $x87_ftw
        local.get $state
        i32.load16_u offset=516 align=1
        local.set $x87_fcw
        local.get $x87_ftw
        local.get $x87_top
        i32.const 1
        i32.shl
        i32.shr_u
        i32.const 3
        i32.and
        i32.const 3
        i32.eq
        if ;; label = @3
          local.get $x87_fcw
          i32.const 1
          i32.and
          i32.eqz
          if ;; label = @4
            local.get $state
            i32.const 2
            i32.store offset=144 align=1
            i64.const 4106
            local.set $rip
            i32.const 2
            local.set $status
            br 3 (;@1;)
          end
          i64.const -4611686018427387904
          local.set $x87_rm
          i32.const 65535
          local.set $x87_rse
          i32.const 65
          local.set $x87_st
        else
          local.get $state
          local.get $x87_top
          i64.extend_i32_u
          i64.const 4
          i64.shl
          i64.add
          i64.load offset=528 align=1
          local.set $x87_am
          local.get $state
          local.get $x87_top
          i64.extend_i32_u
          i64.const 4
          i64.shl
          i64.add
          i32.load16_u offset=536 align=1
          local.set $x87_ase
          local.get $x87_am
          local.set $x87_rm
          local.get $x87_ase
          local.set $x87_rse
          i32.const 0
          local.set $x87_st
        end
        local.get $x87_addr
        local.get $x87_rm
        i64.store align=1
        local.get $x87_addr
        local.get $x87_rse
        i32.store16 offset=8 align=1
        local.get $x87_ftw
        i32.const 3
        local.get $x87_top
        i32.const 1
        i32.shl
        i32.shl
        i32.const -1
        i32.xor
        i32.and
        i32.const 3
        local.get $x87_top
        i32.const 1
        i32.shl
        i32.shl
        i32.or
        local.set $x87_ftw
        local.get $x87_top
        i32.const 1
        i32.add
        i32.const 7
        i32.and
        local.set $x87_top
        local.get $state
        local.get $x87_fsw
        i32.const -14849
        i32.and
        local.get $x87_st
        i32.const 639
        i32.and
        i32.or
        local.get $x87_top
        i32.const 11
        i32.shl
        i32.or
        i32.store16 offset=518 align=1
        local.get $state
        local.get $x87_ftw
        i32.store16 offset=520 align=1
        local.get $cc_a
        local.set $scratch
        local.get $scratch
        i64.const 65
        i64.and
        i64.const 0
        i64.ne
        i32.eqz
        i64.extend_i32_u
        local.set $t0
        local.get $rax
        i64.const -256
        i64.and
        local.get $t0
        i64.or
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
  (@custom "x64wasm.blocks" (after code) "\01\00\00\00\01\00\00\00\01\00\00\00\00\00\00\00\00\10\00\00\00\00\00\00\00\00\00\00\00\00\00\00")
)

