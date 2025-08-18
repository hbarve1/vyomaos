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

/// Main function to test all factorial functions
#[no_mangle]
pub extern "C" fn main() -> i32 {
    println!("🔢 Testing Factorial Functions");
    println!("==============================");
    
    // Test basic factorial function
    println!("📊 Testing factorial(5):");
    let fact5 = factorial(5);
    println!("   ✅ Result: {} (5! = 5×4×3×2×1)", fact5);
    
    println!("📊 Testing factorial(7):");
    let fact7 = factorial(7);
    println!("   ✅ Result: {} (7!)", fact7);
    
    // Test iterative factorial
    println!("🔄 Testing factorial_iterative(6):");
    let fact6_iter = factorial_iterative(6);
    println!("   ✅ Result: {} (6! iterative)", fact6_iter);
    
    println!("🔄 Testing factorial_iterative(8):");
    let fact8_iter = factorial_iterative(8);
    println!("   ✅ Result: {} (8! iterative)", fact8_iter);
    
    // Test double factorial
    println!("📈 Testing double_factorial(5):");
    let dfact5 = double_factorial(5);
    println!("   ✅ Result: {} (5!! = 5×3×1)", dfact5);
    
    println!("📈 Testing double_factorial(6):");
    let dfact6 = double_factorial(6);
    println!("   ✅ Result: {} (6!! = 6×4×2)", dfact6);
    
    // Test rising factorial
    println!("📈 Testing rising_factorial(3, 4):");
    let rfact = rising_factorial(3, 4);
    println!("   ✅ Result: {} (3×4×5×6)", rfact);
    
    println!("📈 Testing rising_factorial(2, 5):");
    let rfact2 = rising_factorial(2, 5);
    println!("   ✅ Result: {} (2×3×4×5×6)", rfact2);
    
    // Test combinations
    println!("🎯 Testing combinations(5, 2):");
    let comb52 = combinations(5, 2);
    println!("   ✅ Result: {} (C(5,2) = 5!/(2!×3!))", comb52);
    
    println!("🎯 Testing combinations(8, 3):");
    let comb83 = combinations(8, 3);
    println!("   ✅ Result: {} (C(8,3))", comb83);
    
    println!("🎯 Testing combinations(10, 0):");
    let comb100 = combinations(10, 0);
    println!("   ✅ Result: {} (C(10,0) = 1)", comb100);
    
    // Test edge cases
    println!("⚠️  Testing edge cases:");
    
    println!("   factorial(0): {}", factorial(0));
    println!("   factorial(1): {}", factorial(1));
    println!("   combinations(5, 6): {} (k > n)", combinations(5, 6));
    
    // Run comprehensive test
    println!("🧪 Running test_calculations():");
    let test_result = test_calculations();
    println!("   ✅ Test sum: {} (expected: 625)", test_result);
    
    if test_result == 625 {
        println!("✨ All factorial function tests PASSED!");
        0 // Return success
    } else {
        println!("❌ Test verification FAILED!");
        1 // Return error
    }
}
