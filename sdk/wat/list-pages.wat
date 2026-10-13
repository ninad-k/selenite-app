(module
  ;; Original ABI v1 example using the default 1 MiB message budget.
  (import "selenite" "call"
    (func $host (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 32 256)
  (data (i32.const 0) "{\22op\22:\22list\22,\22limit\22:100}")
  (func (export "alloc") (param i32) (result i32)
    (i32.const 4096))
  (func (export "run") (param i32 i32) (result i64)
    (i64.or
      (i64.shl (i64.const 65536) (i64.const 32))
      (i64.extend_i32_u
        (call $host
          (i32.const 0) (i32.const 25)
          (i32.const 65536) (i32.const 1048576))))))
