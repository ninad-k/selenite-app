(module
  ;; Original ABI v1 example: return the caller's JSON bytes without imports.
  (memory (export "memory") 32 256)
  (func (export "alloc") (param i32) (result i32)
    (i32.const 4096))
  (func (export "run") (param $pointer i32) (param $length i32) (result i64)
    (i64.or
      (i64.shl (i64.extend_i32_u (local.get $pointer)) (i64.const 32))
      (i64.extend_i32_u (local.get $length)))))
