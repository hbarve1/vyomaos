/// Factorial WebAssembly Application
/// Demonstrates recursive calculations and mathematical operations

/// Calculate factorial recursively
#[no_mangle]
pub extern "C" fn factorial(n: i32) -> i32 {
    if n <= 1 {
        1
    } else {
        n * factorial(n - 1)
    }
}

/// Calculate factorial iteratively (more efficient)
#[no_mangle]
pub extern "C" fn factorial_iterative(n: i32) -> i32 {
    let mut result = 1;
    for i in 2..=n {
        result *= i;
    }
    result
}

/// Calculate double factorial (n!! = n * (n-2) * (n-4) * ...)
#[no_mangle]
pub extern "C" fn double_factorial(n: i32) -> i32 {
    if n <= 1 {
        1
    } else {
        n * double_factorial(n - 2)
    }
}

/// Calculate rising factorial (n^(k) = n * (n+1) * ... * (n+k-1))
#[no_mangle]
pub extern "C" fn rising_factorial(n: i32, k: i32) -> i32 {
    let mut result = 1;
    for i in 0..k {
        result *= n + i;
    }
    result
}

/// Calculate combinations (n choose k)
#[no_mangle]
pub extern "C" fn combinations(n: i32, k: i32) -> i32 {
    if k > n || k < 0 {
        return 0;
    }
    if k == 0 || k == n {
        return 1;
    }
    
    let k = if k > n - k { n - k } else { k }; // Optimization
    
    let mut result = 1;
    for i in 0..k {
        result = result * (n - i) / (i + 1);
    }
    result
}

/// Simple test function
#[no_mangle]
pub extern "C" fn test_calculations() -> i32 {
    // Test basic calculations
    let f5 = factorial(5);           // Should be 120
    let f5_iter = factorial_iterative(5); // Should be 120
    let df5 = double_factorial(5);   // Should be 15 (5*3*1)
    let rf = rising_factorial(3, 4); // Should be 360 (3*4*5*6)
    let c = combinations(5, 2);      // Should be 10
    
    // Return sum as verification
    f5 + f5_iter + df5 + rf + c // Should be 120+120+15+360+10 = 625
}
