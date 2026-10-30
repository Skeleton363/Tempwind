(module
  (type (;0;) (func (param i64) (result i32)))
  (type (;1;) (func (param i32 i64 i64 i64) (result i64)))
  (type (;2;) (func (param i64 i32 i64) (result i32)))
  (type (;3;) (func (param i64) (result i32 i32)))
  (type (;4;) (func (param i64 i64 i64 i32) (result i64 i64 i32)))
  (import "env" "memory" (memory (;0;) i64 1))
  (import "env" "table" (table $table (;0;) 0 funcref))
  (import "env" "table_base" (global $table_base (;0;) i32))
  (import "env" "lookup" (global $lookup (;1;) i64))
  (import "env" "depth" (global $depth (;2;) (mut i32)))
  (export "run" (func $run))
  (elem (;0;) (table $table) (global.get $table_base) func $fn_0x1000)
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
  (func $fn_0x1000 (;3;) (type 2) (param $state i64) (param $entry i32) (param $ret i64) (result i32)
    (local $rax i64) (local $rcx i64) (local $rdx i64) (local $rbx i64) (local $rsp i64) (local $rbp i64) (local $rsi i64) (local $rdi i64) (local $r8 i64) (local $r9 i64) (local $r10 i64) (local $r11 i64) (local $r12 i64) (local $r13 i64) (local $r14 i64) (local $r15 i64) (local $rip i64) (local $cc_a i64) (local $cc_b i64) (local $cc_r i64) (local $scratch i64) (local $t0 i64) (local $t1 i64) (local $t2 i64) (local $t3 i64) (local $t4 i64) (local $t5 i64) (local $t6 i64) (local $t7 i64) (local $t8 i64) (local $t9 i64) (local $t10 i64) (local $t11 i64) (local $t12 i64) (local $t13 i64) (local $t14 i64) (local $cc_op i32) (local $status i32) (local $func i32)
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
        local.get $rdx
        local.get $rax
        local.get $rcx
        i32.const 0
        call $div128
        i64.extend_i32_u
        local.set $t2
        local.set $t1
        local.set $t0
        local.get $t2
        i64.const 0
        i64.ne
        if ;; label = @3
          local.get $state
          i32.const 4
          i32.store offset=200 align=1
          local.get $state
          i64.const 0
          i64.store offset=192 align=1
          local.get $state
          i32.const 3
          i32.store offset=144 align=1
          i64.const 4096
          local.set $rip
          i32.const 2
          local.set $status
          br 2 (;@1;)
        end
        local.get $t0
        local.set $rax
        local.get $t1
        local.set $rdx
        local.get $rbx
        i64.const 255
        i64.and
        local.set $t0
        local.get $rax
        i64.const 65535
        i64.and
        i64.extend16_s
        local.set $t2
        local.get $t0
        i64.extend8_s
        local.set $t3
        local.get $t3
        i64.const 0
        i64.eq
        i64.extend_i32_u
        i64.const 0
        i64.ne
        if ;; label = @3
          local.get $state
          i32.const 4
          i32.store offset=200 align=1
          local.get $state
          i64.const 0
          i64.store offset=192 align=1
          local.get $state
          i32.const 3
          i32.store offset=144 align=1
          i64.const 4099
          local.set $rip
          i32.const 2
          local.set $status
          br 2 (;@1;)
        end
        local.get $t2
        local.get $t3
        i64.div_s
        local.set $t5
        local.get $t5
        i64.extend8_s
        local.set $t6
        local.get $t5
        local.get $t6
        i64.ne
        i64.extend_i32_u
        i64.const 0
        i64.ne
        if ;; label = @3
          local.get $state
          i32.const 4
          i32.store offset=200 align=1
          local.get $state
          i64.const 0
          i64.store offset=192 align=1
          local.get $state
          i32.const 3
          i32.store offset=144 align=1
          i64.const 4099
          local.set $rip
          i32.const 2
          local.set $status
          br 2 (;@1;)
        end
        local.get $t2
        local.get $t3
        i64.rem_s
        local.set $t8
        local.get $t5
        i64.const 255
        i64.and
        local.set $t9
        local.get $t8
        i64.const 255
        i64.and
        i64.const 8
        i64.shl
        local.get $t9
        i64.or
        local.set $t12
        local.get $rax
        i64.const -65536
        i64.and
        local.get $t12
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
  (func $div128 (;4;) (type 4) (param $hi i64) (param $lo i64) (param $d i64) (param $signed i32) (result i64 i64 i32)
    (local $q i64) (local $r i64) (local $s i64) (local $vn1 i64) (local $vn0 i64) (local $un32 i64) (local $un10 i64) (local $un1 i64) (local $un0 i64) (local $q1 i64) (local $q0 i64) (local $rhat i64) (local $un21 i64) (local $neg_n i32) (local $neg_d i32)
    local.get $signed
    if ;; label = @1
      local.get $hi
      i64.const 0
      i64.lt_s
      local.tee $neg_n
      if ;; label = @2
        local.get $hi
        i64.const -1
        i64.xor
        local.get $lo
        i64.eqz
        i64.extend_i32_u
        i64.add
        local.set $hi
        i64.const 0
        local.get $lo
        i64.sub
        local.set $lo
      end
      local.get $d
      i64.const 0
      i64.lt_s
      local.tee $neg_d
      if ;; label = @2
        i64.const 0
        local.get $d
        i64.sub
        local.set $d
      end
    end
    local.get $d
    i64.eqz
    local.get $hi
    local.get $d
    i64.ge_u
    i32.or
    if ;; label = @1
      i64.const 0
      i64.const 0
      i32.const 1
      return
    end
    local.get $hi
    i64.eqz
    if ;; label = @1
      local.get $lo
      local.get $d
      i64.div_u
      local.set $q
      local.get $lo
      local.get $d
      i64.rem_u
      local.set $r
    else
      local.get $d
      i64.clz
      local.tee $s
      local.get $d
      local.get $s
      i64.shl
      local.set $d
      drop
      local.get $d
      i64.const 32
      i64.shr_u
      local.set $vn1
      local.get $d
      i64.const 4294967295
      i64.and
      local.set $vn0
      local.get $hi
      local.get $s
      i64.shl
      local.get $lo
      i64.const 64
      local.get $s
      i64.sub
      i64.shr_u
      i64.const 0
      local.get $s
      i64.sub
      i64.const 63
      i64.shr_s
      i64.and
      i64.or
      local.set $un32
      local.get $lo
      local.get $s
      i64.shl
      local.tee $un10
      i64.const 32
      i64.shr_u
      local.set $un1
      local.get $un10
      i64.const 4294967295
      i64.and
      local.set $un0
      local.get $un32
      local.get $vn1
      i64.div_u
      local.tee $q1
      local.get $vn1
      i64.mul
      local.set $rhat
      local.get $un32
      local.get $rhat
      i64.sub
      local.set $rhat
      local.get $q1
      i64.const 4294967296
      i64.ge_u
      local.get $q1
      local.get $vn0
      i64.mul
      local.get $rhat
      i64.const 32
      i64.shl
      local.get $un1
      i64.add
      i64.gt_u
      i32.or
      if ;; label = @2
        local.get $q1
        i64.const 1
        i64.sub
        local.set $q1
        local.get $rhat
        local.get $vn1
        i64.add
        local.set $rhat
        local.get $rhat
        i64.const 4294967296
        i64.lt_u
        if ;; label = @3
          local.get $q1
          i64.const 4294967296
          i64.ge_u
          local.get $q1
          local.get $vn0
          i64.mul
          local.get $rhat
          i64.const 32
          i64.shl
          local.get $un1
          i64.add
          i64.gt_u
          i32.or
          if ;; label = @4
            local.get $q1
            i64.const 1
            i64.sub
            local.set $q1
            local.get $rhat
            local.get $vn1
            i64.add
            local.set $rhat
          end
        end
      end
      local.get $un32
      i64.const 32
      i64.shl
      local.get $un1
      i64.add
      local.get $q1
      local.get $d
      i64.mul
      i64.sub
      local.set $un21
      local.get $un21
      local.get $vn1
      i64.div_u
      local.tee $q0
      local.get $vn1
      i64.mul
      local.set $rhat
      local.get $un21
      local.get $rhat
      i64.sub
      local.set $rhat
      local.get $q0
      i64.const 4294967296
      i64.ge_u
      local.get $q0
      local.get $vn0
      i64.mul
      local.get $rhat
      i64.const 32
      i64.shl
      local.get $un0
      i64.add
      i64.gt_u
      i32.or
      if ;; label = @2
        local.get $q0
        i64.const 1
        i64.sub
        local.set $q0
        local.get $rhat
        local.get $vn1
        i64.add
        local.set $rhat
        local.get $rhat
        i64.const 4294967296
        i64.lt_u
        if ;; label = @3
          local.get $q0
          i64.const 4294967296
          i64.ge_u
          local.get $q0
          local.get $vn0
          i64.mul
          local.get $rhat
          i64.const 32
          i64.shl
          local.get $un0
          i64.add
          i64.gt_u
          i32.or
          if ;; label = @4
            local.get $q0
            i64.const 1
            i64.sub
            local.set $q0
            local.get $rhat
            local.get $vn1
            i64.add
            local.set $rhat
          end
        end
      end
      local.get $un21
      i64.const 32
      i64.shl
      local.get $un0
      i64.add
      local.get $q0
      local.get $d
      i64.mul
      i64.sub
      local.get $s
      i64.shr_u
      local.set $r
      local.get $q1
      i64.const 32
      i64.shl
      local.get $q0
      i64.add
      local.set $q
    end
    local.get $signed
    if ;; label = @1
      local.get $neg_n
      local.get $neg_d
      i32.ne
      if ;; label = @2
        local.get $q
        i64.const -9223372036854775808
        i64.gt_u
        if ;; label = @3
          i64.const 0
          i64.const 0
          i32.const 1
          return
        end
        i64.const 0
        local.get $q
        i64.sub
        local.set $q
      else
        local.get $q
        i64.const 0
        i64.lt_s
        if ;; label = @3
          i64.const 0
          i64.const 0
          i32.const 1
          return
        end
      end
      local.get $neg_n
      if ;; label = @2
        i64.const 0
        local.get $r
        i64.sub
        local.set $r
      end
    end
    local.get $q
    local.get $r
    i32.const 0
  )
  (@custom "x64wasm.blocks" (after code) "\01\00\00\00\01\00\00\00\01\00\00\00\00\00\00\00\00\10\00\00\00\00\00\00\00\00\00\00\00\00\00\00")
)

