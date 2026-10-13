//! Original dependency-free ABI v1 guest. Each invocation has a fresh instance;
//! allocations live only for that invocation and are dropped with its store.

#[unsafe(no_mangle)]
pub extern "C" fn alloc(length: i32) -> i32 {
    if !(0..=2 * 1024 * 1024).contains(&length) {
        return -1;
    }
    let buffer = vec![0_u8; length as usize].into_boxed_slice();
    Box::into_raw(buffer) as *mut u8 as usize as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn run(pointer: i32, length: i32) -> i64 {
    // The host supplied these bytes and checked their bounds. Echoing them
    // needs no raw-pointer dereference or host capability.
    (((pointer as u32 as u64) << 32) | length as u32 as u64) as i64
}
