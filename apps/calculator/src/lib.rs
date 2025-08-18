/// Basic arithmetic operations for WebAssembly

#[no_mangle]
pub extern "C" fn add(a: f64, b: f64) -> f64 {
    a + b
}

#[no_mangle]
pub extern "C" fn subtract(a: f64, b: f64) -> f64 {
    a - b
}

#[no_mangle]
pub extern "C" fn multiply(a: f64, b: f64) -> f64 {
    a * b
}

#[no_mangle]
pub extern "C" fn divide(a: f64, b: f64) -> f64 {
    if b != 0.0 {
        a / b
    } else {
        f64::NAN
    }
}

#[no_mangle]
pub extern "C" fn power(base: f64, exponent: f64) -> f64 {
    base.powf(exponent)
}

#[no_mangle]
pub extern "C" fn sqrt(x: f64) -> f64 {
    x.sqrt()
}

#[no_mangle]
pub extern "C" fn sin(x: f64) -> f64 {
    x.sin()
}

#[no_mangle]
pub extern "C" fn cos(x: f64) -> f64 {
    x.cos()
}

#[no_mangle]
pub extern "C" fn log(x: f64) -> f64 {
    x.ln()
}

/// Main function to test all calculator functions
#[no_mangle]
pub extern "C" fn main() -> i32 {
    println!("🧮 Testing Calculator Functions");
    println!("===============================");
    
    // Test basic arithmetic operations
    println!("➕ Testing add(25.5, 14.3):");
    let sum = add(25.5, 14.3);
    println!("   ✅ Result: {:.2}", sum);
    
    println!("➖ Testing subtract(100.0, 35.7):");
    let diff = subtract(100.0, 35.7);
    println!("   ✅ Result: {:.2}", diff);
    
    println!("✖️  Testing multiply(7.5, 8.0):");
    let product = multiply(7.5, 8.0);
    println!("   ✅ Result: {:.2}", product);
    
    println!("➗ Testing divide(144.0, 12.0):");
    let quotient = divide(144.0, 12.0);
    println!("   ✅ Result: {:.2}", quotient);
    
    println!("➗ Testing divide(10.0, 0.0) [division by zero]:");
    let nan_result = divide(10.0, 0.0);
    if nan_result.is_nan() {
        println!("   ✅ Result: NaN (correctly handled division by zero)");
    } else {
        println!("   ⚠️  Result: {:.2}", nan_result);
    }
    
    // Test advanced functions
    println!("🔢 Testing power(2.0, 8.0):");
    let pow_result = power(2.0, 8.0);
    println!("   ✅ Result: {:.2}", pow_result);
    
    println!("√ Testing sqrt(144.0):");
    let sqrt_result = sqrt(144.0);
    println!("   ✅ Result: {:.2}", sqrt_result);
    
    println!("📐 Testing sin(π/2) [sin(1.5708)]:");
    let sin_result = sin(std::f64::consts::PI / 2.0);
    println!("   ✅ Result: {:.4}", sin_result);
    
    println!("📐 Testing cos(0.0):");
    let cos_result = cos(0.0);
    println!("   ✅ Result: {:.4}", cos_result);
    
    println!("📊 Testing log(e) [log(2.71828)]:");
    let log_result = log(std::f64::consts::E);
    println!("   ✅ Result: {:.4}", log_result);
    
    println!("✨ All calculator function tests completed!");
    
    0 // Return success
}
