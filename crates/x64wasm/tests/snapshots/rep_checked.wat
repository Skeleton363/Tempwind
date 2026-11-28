(module
  (type (;0;) (func (param i64) (result i32)))
  (type (;1;) (func (param i32 i64 i64 i64) (result i64)))
  (type (;2;) (func (param i64 i32 i64) (result i32)))
  (type (;3;) (func (param i64) (result i32 i32)))
  (import "env" "memory" (memory (;0;) i64 1))
  (import "env" "table" (table $table (;0;) 0 funcref))
  (import "env" "table_base" (global $table_base (;0;) i32))
  (import "env" "lookup" (global $lookup (;1;) i64))
  (import "env" "depth" (global $depth (;2;) (mut i32)))
  (import "env" "page_flags" (global $page_flags (;3;) i64))
  (export "run" (func $run))
  (elem (;0;) (table $table) (global.get $table_base) func $fn_0x1000)
  (func $run (;0;) (type 0) (param $state i64) (result i32)
    (local $func i32) (local $entry i32) (local $status i32) (local $d0 i32)
    local.get $state
    i32.const 0
    i32.store offset=148 align=1
    local.get $state
    local.get $state
    i64.load offset=136 align=1
    i64.store offset=152 align=1
    global.get $depth
    local.set $d0
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
        local.get $d0
        global.set $depth
        br 0 (;@2;)
      end
    end
    local.get $d0
    global.set $depth
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
    (local $rax i64) (local $rcx i64) (local $rdx i64) (local $rbx i64) (local $rsp i64) (local $rbp i64) (local $rsi i64) (local $rdi i64) (local $r8 i64) (local $r9 i64) (local $r10 i64) (local $r11 i64) (local $r12 i64) (local $r13 i64) (local $r14 i64) (local $r15 i64) (local $rip i64) (local $cc_a i64) (local $cc_b i64) (local $cc_r i64) (local $scratch i64) (local $t0 i64) (local $t1 i64) (local $t2 i64) (local $t3 i64) (local $t4 i64) (local $t5 i64) (local $t6 i64) (local $t7 i64) (local $t8 i64) (local $cc_op i32) (local $status i32) (local $func i32) (local $mask i32)
    local.get $state
    i64.load align=1
    local.set $rax
    local.get $state
    i64.load offset=8 align=1
    local.set $rcx
    local.get $state
    i64.load offset=32 align=1
    local.set $rsp
    local.get $state
    i64.load offset=48 align=1
    local.set $rsi
    local.get $state
    i64.load offset=56 align=1
    local.set $rdi
    block ;; label = @1
      block ;; label = @2
        block ;; label = @3
          block ;; label = @4
            local.get $entry
            br_table 1 (;@3;) 0 (;@4;)
          end
          unreachable
        end
        local.get $state
        i64.load offset=136 align=1
        i64.const 1024
        i64.or
        local.set $t1
        local.get $state
        local.get $t1
        i64.store offset=136 align=1
        local.get $state
        i64.load offset=136 align=1
        i64.const 1024
        i64.and
        local.set $t1
        i64.const -8
        i64.const 8
        local.get $t1
        i64.const 0
        i64.ne
        select
        local.set $t2
        local.get $rcx
        i64.const 1
        i64.sub
        local.get $t2
        i64.mul
        local.set $t4
        local.get $rdi
        local.get $t4
        i64.add
        local.set $t5
        local.get $rcx
        i64.const 0
        i64.ne
        if ;; label = @3
          local.get $rdi
          i64.const 65536
          i64.lt_u
          if ;; label = @4
            local.get $rdi
            local.set $scratch
            i32.const 5
            local.set $status
            i32.const 0
            local.set $mask
            br 2 (;@2;)
          end
        end
        local.get $rcx
        i64.const 0
        i64.ne
        if ;; label = @3
          local.get $t5
          i64.const 65536
          i64.lt_u
          if ;; label = @4
            local.get $t5
            local.set $scratch
            i32.const 5
            local.set $status
            i32.const 0
            local.set $mask
            br 2 (;@2;)
          end
        end
        local.get $t5
        local.get $rdi
        local.get $t1
        i64.const 0
        i64.ne
        select
        local.set $t6
        local.get $rdi
        local.get $t5
        local.get $t1
        i64.const 0
        i64.ne
        select
        i64.const 7
        i64.add
        local.set $t8
        local.get $rcx
        i64.const 0
        i64.ne
        if ;; label = @3
          local.get $t6
          local.set $scratch
          block ;; label = @4
            loop ;; label = @5
              local.get $scratch
              i64.const 17179869184
              i64.ge_u
              if (result i32) ;; label = @6
                i32.const 1
              else
                global.get $page_flags
                local.get $scratch
                i64.const 12
                i64.shr_u
                i64.add
                i32.load8_u
                i32.const 3
                i32.and
              end
              if ;; label = @6
                i32.const 6
                local.set $status
                i32.const 0
                local.set $mask
                br 4 (;@2;)
              end
              local.get $scratch
              i64.const 12
              i64.shr_u
              local.get $t8
              i64.const 12
              i64.shr_u
              i64.ge_u
              br_if 1 (;@4;)
              local.get $scratch
              i64.const 4095
              i64.or
              i64.const 1
              i64.add
              local.set $scratch
              br 0 (;@5;)
            end
          end
        end
        local.get $t2
        local.set $scratch
        block ;; label = @3
          loop ;; label = @4
            local.get $rcx
            i64.eqz
            br_if 1 (;@3;)
            local.get $rdi
            local.get $rax
            i64.store align=1
            local.get $rdi
            local.get $scratch
            i64.add
            local.set $rdi
            local.get $rcx
            i64.const 1
            i64.sub
            local.set $rcx
            br 0 (;@4;)
          end
        end
        local.get $state
        i64.load offset=136 align=1
        i64.const -1025
        i64.and
        local.set $t1
        local.get $state
        local.get $t1
        i64.store offset=136 align=1
        local.get $state
        i64.load offset=136 align=1
        i64.const 1024
        i64.and
        local.set $t1
        i64.const -1
        i64.const 1
        local.get $t1
        i64.const 0
        i64.ne
        select
        local.set $t2
        local.get $rcx
        i64.const 1
        i64.sub
        local.get $t2
        i64.mul
        local.set $t4
        local.get $rsi
        local.get $t4
        i64.add
        local.set $t5
        local.get $rcx
        i64.const 0
        i64.ne
        if ;; label = @3
          local.get $rsi
          i64.const 65536
          i64.lt_u
          if ;; label = @4
            local.get $rsi
            local.set $scratch
            i32.const 20
            local.set $status
            i32.const 130
            local.set $mask
            br 2 (;@2;)
          end
        end
        local.get $rcx
        i64.const 0
        i64.ne
        if ;; label = @3
          local.get $t5
          i64.const 65536
          i64.lt_u
          if ;; label = @4
            local.get $t5
            local.set $scratch
            i32.const 20
            local.set $status
            i32.const 130
            local.set $mask
            br 2 (;@2;)
          end
        end
        local.get $rdi
        local.get $t4
        i64.add
        local.set $t6
        local.get $rcx
        i64.const 0
        i64.ne
        if ;; label = @3
          local.get $rdi
          i64.const 65536
          i64.lt_u
          if ;; label = @4
            local.get $rdi
            local.set $scratch
            i32.const 21
            local.set $status
            i32.const 130
            local.set $mask
            br 2 (;@2;)
          end
        end
        local.get $rcx
        i64.const 0
        i64.ne
        if ;; label = @3
          local.get $t6
          i64.const 65536
          i64.lt_u
          if ;; label = @4
            local.get $t6
            local.set $scratch
            i32.const 21
            local.set $status
            i32.const 130
            local.set $mask
            br 2 (;@2;)
          end
        end
        local.get $t6
        local.get $rdi
        local.get $t1
        i64.const 0
        i64.ne
        select
        local.set $t7
        local.get $rdi
        local.get $t6
        local.get $t1
        i64.const 0
        i64.ne
        select
        local.set $t8
        local.get $rcx
        i64.const 0
        i64.ne
        if ;; label = @3
          local.get $t7
          local.set $scratch
          block ;; label = @4
            loop ;; label = @5
              local.get $scratch
              i64.const 17179869184
              i64.ge_u
              if (result i32) ;; label = @6
                i32.const 1
              else
                global.get $page_flags
                local.get $scratch
                i64.const 12
                i64.shr_u
                i64.add
                i32.load8_u
                i32.const 3
                i32.and
              end
              if ;; label = @6
                i32.const 22
                local.set $status
                i32.const 130
                local.set $mask
                br 4 (;@2;)
              end
              local.get $scratch
              i64.const 12
              i64.shr_u
              local.get $t8
              i64.const 12
              i64.shr_u
              i64.ge_u
              br_if 1 (;@4;)
              local.get $scratch
              i64.const 4095
              i64.or
              i64.const 1
              i64.add
              local.set $scratch
              br 0 (;@5;)
            end
          end
        end
        local.get $t2
        local.set $scratch
        block ;; label = @3
          local.get $scratch
          i64.const 0
          i64.gt_s
          local.get $rcx
          i64.const 1099511627776
          i64.lt_u
          i32.and
          local.get $rdi
          local.get $rsi
          i64.sub
          local.get $rcx
          i64.const 1
          i64.mul
          i64.ge_u
          i32.and
          if ;; label = @4
            local.get $rcx
            i64.const 1
            i64.mul
            local.set $scratch
            local.get $rdi
            local.get $rsi
            local.get $scratch
            memory.copy
            local.get $rsi
            local.get $scratch
            i64.add
            local.set $rsi
            local.get $rdi
            local.get $scratch
            i64.add
            local.set $rdi
            i64.const 0
            local.set $rcx
            br 1 (;@3;)
          end
          loop ;; label = @4
            local.get $rcx
            i64.eqz
            br_if 1 (;@3;)
            local.get $rdi
            local.get $rsi
            i64.load8_u
            i64.store8
            local.get $rsi
            local.get $scratch
            i64.add
            local.set $rsi
            local.get $rdi
            local.get $scratch
            i64.add
            local.set $rdi
            local.get $rcx
            i64.const 1
            i64.sub
            local.set $rcx
            br 0 (;@4;)
          end
        end
        local.get $rsp
        i64.const 65536
        i64.lt_u
        if ;; label = @3
          local.get $rsp
          local.set $scratch
          i32.const 28
          local.set $status
          i32.const 194
          local.set $mask
          br 1 (;@2;)
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
        i32.const 210
        local.set $mask
        br 1 (;@1;)
        unreachable
      end
      local.get $status
      i32.const 2
      i32.shr_u
      i64.extend_i32_u
      i64.const 4096
      i64.add
      local.set $rip
      local.get $state
      i32.const 8
      local.get $status
      i32.const 1
      i32.and
      i32.const 1
      i32.add
      local.get $status
      i32.const 2
      i32.and
      select
      i32.store offset=200 align=1
      local.get $state
      local.get $scratch
      i64.store offset=192 align=1
      local.get $state
      i32.const 3
      i32.store offset=144 align=1
      i32.const 2
      local.set $status
    end
    local.get $mask
    i32.const 130
    i32.and
    if ;; label = @1
      local.get $state
      local.get $rcx
      i64.store offset=8 align=1
      local.get $state
      local.get $rdi
      i64.store offset=56 align=1
    end
    local.get $mask
    i32.const 16
    i32.and
    if ;; label = @1
      local.get $state
      local.get $rsp
      i64.store offset=32 align=1
    end
    local.get $mask
    i32.const 64
    i32.and
    if ;; label = @1
      local.get $state
      local.get $rsi
      i64.store offset=48 align=1
    end
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

