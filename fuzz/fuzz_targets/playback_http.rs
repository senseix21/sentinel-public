use std::ffi::{c_char, c_int, CString};

unsafe extern "C" {
    fn LLVMFuzzerRunDriver(
        argc: *mut c_int,
        argv: *mut *mut *mut c_char,
        callback: unsafe extern "C" fn(*const u8, usize) -> c_int,
    ) -> c_int;
    fn LLVMFuzzerTestOneInput(data: *const u8, size: usize) -> c_int;
}

// A native codec bundled by WebRTC also exports main on macOS. A Rust-owned
// entry point prevents that archive member from stealing libFuzzer's startup.
fn main() {
    let mut arguments: Vec<Vec<u8>> = std::env::args_os()
        .map(|argument| {
            CString::new(argument.as_encoded_bytes())
                .expect("OS arguments have no NUL")
                .into_bytes_with_nul()
        })
        .collect();
    let mut argc = c_int::try_from(arguments.len()).expect("OS argument count fits c_int");
    let mut pointers: Vec<*mut c_char> = arguments
        .iter_mut()
        .map(|argument| argument.as_mut_ptr().cast())
        .chain(std::iter::once(std::ptr::null_mut()))
        .collect();
    let mut argv = pointers.as_mut_ptr();
    // SAFETY: argv is a NUL-terminated array of argc live, writable C strings.
    // The backing byte buffers and pointer vector live for the entire driver
    // call. The callback has libFuzzer's exact C ABI and is
    // generated below by libfuzzer_sys. This FFI exists only in the fuzz binary.
    let status = unsafe { LLVMFuzzerRunDriver(&mut argc, &mut argv, LLVMFuzzerTestOneInput) };
    std::process::exit(status);
}

libfuzzer_sys::fuzz_target!(|input: &[u8]| {
    surveillance_server::health_http::fuzz_playback_request(input);
});
