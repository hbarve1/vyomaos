use std::ffi::CString;
use std::os::raw::c_char;

/// Export a function that can be called from the WebAssembly host
#[no_mangle]
pub extern "C" fn hello() -> *mut c_char {
    let message = CString::new("Hello from Rust WebAssembly!").unwrap();
    message.into_raw()
}

/// Export a function that adds two numbers
#[no_mangle]
pub extern "C" fn add(a: i32, b: i32) -> i32 {
    a + b
}

/// Export a function that calculates factorial
#[no_mangle]
pub extern "C" fn factorial(n: i32) -> i32 {
    if n <= 1 {
        1
    } else {
        n * factorial(n - 1)
    }
}

/// Memory management - free strings allocated by hello()
#[no_mangle]
pub extern "C" fn free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe {
            let _ = CString::from_raw(ptr);
        }
    }
}
