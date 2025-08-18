use std::ffi::CString;
use std::os::raw::c_char;

/// Export a function that can be called from the WebAssembly host
#[no_mangle]
pub extern "C" fn hello() -> *mut c_char {
    let message = CString::new("Hello from Rust WebAssembly! 123 123 123 123").unwrap();
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

/// Main function to test all exported functions
#[no_mangle]
pub extern "C" fn main() -> i32 {
    println!("🧪 Testing WebAssembly Functions");
    println!("================================");
    
    // Test hello function
    println!("📞 Testing hello():");
    let hello_ptr = hello();
    if !hello_ptr.is_null() {
        unsafe {
            let hello_cstr = std::ffi::CStr::from_ptr(hello_ptr);
            if let Ok(hello_str) = hello_cstr.to_str() {
                println!("   ✅ Result: {}", hello_str);
            }
        }
        free_string(hello_ptr);
    }
    
    // Test add function
    println!("🔢 Testing add(15, 27):");
    let sum = add(15, 27);
    println!("   ✅ Result: {}", sum);
    
    // Test factorial function
    println!("🔢 Testing factorial(5):");
    let fact = factorial(5);
    println!("   ✅ Result: {}", fact);
    
    // Test factorial function with different values
    println!("🔢 Testing factorial(7):");
    let fact2 = factorial(7);
    println!("   ✅ Result: {}", fact2);
    
    // Test add function with different values
    println!("🔢 Testing add(100, 200):");
    let sum2 = add(100, 200);
    println!("   ✅ Result: {}", sum2);
    
    println!("✨ All function tests completed!");
    
    0 // Return success
}
