# Utilities

Helper functions and utilities including WebAssembly text format conversion, table management, global variable handling, development aids, and convenience functions for common WebAssembly operations and debugging tasks.

## Capabilities

### Text Format Conversion

WebAssembly Text (WAT) to binary (WASM) conversion utility providing human-readable WebAssembly format support for development, testing, and educational purposes.

```python { .api }
def wat2wasm(wat: str) -> bytes:
    """
    Convert WebAssembly Text format to binary format.
    
    Parameters:
    - wat: WebAssembly text format string
    
    Returns:
    Binary WebAssembly module bytes
    
    Raises:
    WasmtimeError: If WAT parsing or compilation fails
    """
```

### Table Management

WebAssembly table objects providing reference storage, dynamic table operations, element access, and table growth capabilities for function references and external references.

```python { .api }
class Table:
    def __init__(self, store: Store, ty: TableType, init):
        """
        Create a new table with specified type and initial value.
        
        Parameters:
        - store: Store for table allocation context
        - ty: Table type specifying element type and size limits
        - init: Initial value for all table elements
        
        Raises:
        WasmtimeError: If table creation fails
        """
    
    def type(self, store: Store) -> TableType:
        """
        Get the table type specification.
        
        Parameters:
        - store: Store context
        
        Returns:
        Table type with element type and current size limits
        """
    
    def size(self, store: Store) -> int:
        """
        Get current table size in elements.
        
        Parameters:
        - store: Store context
        
        Returns:
        Number of elements in the table
        """
    
    def grow(self, store: Store, delta: int, init) -> int:
        """
        Grow table by specified number of elements.
        
        Parameters:
        - store: Store context
        - delta: Number of elements to add (must be non-negative)
        - init: Initial value for new elements
        
        Returns:
        Previous table size in elements
        
        Raises:
        WasmtimeError: If growth fails or exceeds maximum size
        """
    
    def get(self, store: Store, idx: int):
        """
        Get element at specified index.
        
        Parameters:
        - store: Store context
        - idx: Element index (zero-based)
        
        Returns:
        Element value at the specified index
        
        Raises:
        WasmtimeError: If index is out of bounds
        """
    
    def set(self, store: Store, idx: int, val) -> None:
        """
        Set element at specified index.
        
        Parameters:
        - store: Store context
        - idx: Element index (zero-based)
        - val: New value for the element
        
        Raises:
        WasmtimeError: If index is out of bounds or value type mismatch
        """
```

### Global Variables

WebAssembly global variable objects providing mutable and immutable global state, type-safe value access, and runtime global variable management for WebAssembly modules.

```python { .api }
class Global:
    def __init__(self, store: Store, ty: GlobalType, val: Val):
        """
        Create a new global variable with specified type and initial value.
        
        Parameters:
        - store: Store for global allocation context
        - ty: Global type specifying value type and mutability
        - val: Initial value for the global variable
        
        Raises:
        WasmtimeError: If global creation fails or value type mismatch
        """
    
    def type(self, store: Store) -> GlobalType:
        """
        Get the global variable type specification.
        
        Parameters:
        - store: Store context
        
        Returns:
        Global type with value type and mutability information
        """
    
    def value(self, store: Store) -> Val:
        """
        Get the current value of the global variable.
        
        Parameters:
        - store: Store context
        
        Returns:
        Current global variable value
        """
    
    def set_value(self, store: Store, val: Val) -> None:
        """
        Set the value of the global variable (if mutable).
        
        Parameters:
        - store: Store context
        - val: New value for the global variable
        
        Raises:
        WasmtimeError: If global is immutable or value type mismatch
        """
```

## Usage Examples

### WAT to WASM Conversion

```python
import wasmtime

# Simple arithmetic module in WAT format
wat_code = '''
(module
  (func (export "add") (param i32 i32) (result i32)
    local.get 0
    local.get 1
    i32.add)
  (func (export "multiply") (param f32 f32) (result f32)
    local.get 0
    local.get 1
    f32.mul)
  (memory (export "memory") 1)
  (global (export "counter") (mut i32) (i32.const 0))
)
'''

# Convert WAT to WASM binary
try:
    wasm_bytes = wasmtime.wat2wasm(wat_code)
    print(f"Converted WAT to WASM: {len(wasm_bytes)} bytes")
    
    # Use the compiled module
    engine = wasmtime.Engine()
    store = wasmtime.Store(engine)
    module = wasmtime.Module(engine, wasm_bytes)
    instance = wasmtime.Instance(store, module, [])
    
    # Call the functions
    add_func = instance.exports(store)["add"]
    multiply_func = instance.exports(store)["multiply"]
    
    result1 = add_func(store, 15, 27)
    result2 = multiply_func(store, 3.5, 2.0)
    
    print(f"15 + 27 = {result1}")
    print(f"3.5 * 2.0 = {result2}")
    
except wasmtime.WasmtimeError as e:
    print(f"WAT compilation failed: {e}")
```

### Complex WAT Example with Tables and Globals

```python
import wasmtime

# Advanced WAT example with tables and indirect calls
advanced_wat = '''
(module
  ;; Function type for binary operations
  (type $binary_op (func (param i32 i32) (result i32)))
  
  ;; Function table for indirect calls
  (table (export "functions") 4 funcref)
  
  ;; Global counter
  (global $counter (mut i32) (i32.const 0))
  (global (export "counter") (mut i32) (i32.const 0))
  
  ;; Binary operation functions
  (func $add (type $binary_op)
    local.get 0
    local.get 1
    i32.add)
  
  (func $sub (type $binary_op)
    local.get 0
    local.get 1
    i32.sub)
  
  (func $mul (type $binary_op)
    local.get 0
    local.get 1
    i32.mul)
  
  (func $div (type $binary_op)
    local.get 0
    local.get 1
    i32.div_s)
  
  ;; Initialize function table
  (elem (i32.const 0) $add $sub $mul $div)
  
  ;; Indirect call function
  (func (export "call_op") (param i32 i32 i32) (result i32)
    ;; Increment counter
    global.get $counter
    i32.const 1
    i32.add
    global.set $counter
    
    ;; Indirect call
    local.get 1  ;; first operand
    local.get 2  ;; second operand
    local.get 0  ;; function index
    call_indirect (type $binary_op))
  
  ;; Get counter value
  (func (export "get_counter") (result i32)
    global.get $counter)
)
'''

# Convert and use the advanced module
try:
    wasm_bytes = wasmtime.wat2wasm(advanced_wat)
    
    engine = wasmtime.Engine()
    store = wasmtime.Store(engine)
    module = wasmtime.Module(engine, wasm_bytes)
    instance = wasmtime.Instance(store, module, [])
    
    exports = instance.exports(store)
    call_op = exports["call_op"]
    get_counter = exports["get_counter"]
    functions_table = exports["functions"]
    counter_global = exports["counter"]
    
    # Test indirect function calls
    operations = ["add", "sub", "mul", "div"]
    for i, op_name in enumerate(operations):
        result = call_op(store, i, 20, 4)  # function_index, a, b
        counter = get_counter(store)
        print(f"{op_name}(20, 4) = {result}, counter = {counter}")
    
    # Access global directly
    final_counter = counter_global.value(store)
    print(f"Final counter value: {final_counter.value}")
    
except wasmtime.WasmtimeError as e:
    print(f"Advanced WAT compilation failed: {e}")
```

### Table Operations

```python
import wasmtime

# Create engine and store
engine = wasmtime.Engine()
store = wasmtime.Store(engine)

# Create table type for function references (0-10 elements)
table_limits = wasmtime.Limits(0, 10)
table_type = wasmtime.TableType(wasmtime.ValType.FUNCREF, table_limits)

# Create table with null initial value
table = wasmtime.Table(store, table_type, wasmtime.Val.funcref(None))

print(f"Created table with {table.size(store)} elements")

# Create some functions to store in the table
def host_func1(x: int) -> int:
    return x * 2

def host_func2(x: int) -> int:
    return x + 10

func_type = wasmtime.FuncType([wasmtime.ValType.I32], [wasmtime.ValType.I32])
func1 = wasmtime.Func(store, func_type, host_func1)
func2 = wasmtime.Func(store, func_type, host_func2)

# Grow table to accommodate functions
old_size = table.grow(store, 2, wasmtime.Val.funcref(None))
print(f"Table grown from {old_size} to {table.size(store)} elements")

# Set functions in table
table.set(store, 0, func1)
table.set(store, 1, func2)

print("Functions stored in table")

# Retrieve and call functions from table
for i in range(2):
    func_val = table.get(store, i)
    if func_val.value is not None:
        func = func_val.value
        result = func(store, 5)
        print(f"Function at index {i}: f(5) = {result}")

# Try to access out of bounds (will raise error)
try:
    table.get(store, 10)
except wasmtime.WasmtimeError as e:
    print(f"Expected error for out of bounds access: {e}")
```

### Global Variable Management

```python
import wasmtime

# Create engine and store
engine = wasmtime.Engine()
store = wasmtime.Store(engine)

# Create mutable global variables
mutable_i32_type = wasmtime.GlobalType(wasmtime.ValType.I32, True)
mutable_global = wasmtime.Global(store, mutable_i32_type, wasmtime.Val.i32(42))

# Create immutable global variable
immutable_f64_type = wasmtime.GlobalType(wasmtime.ValType.F64, False)
immutable_global = wasmtime.Global(store, immutable_f64_type, wasmtime.Val.f64(3.14159))

# Read global values
mutable_value = mutable_global.value(store)
immutable_value = immutable_global.value(store)

print(f"Mutable global: {mutable_value.value} (type: {mutable_value.type})")
print(f"Immutable global: {immutable_value.value} (type: {immutable_value.type})")

# Modify mutable global
print("Incrementing mutable global...")
current_value = mutable_global.value(store)
new_value = wasmtime.Val.i32(current_value.value + 1)
mutable_global.set_value(store, new_value)

updated_value = mutable_global.value(store)
print(f"Updated mutable global: {updated_value.value}")

# Try to modify immutable global (will raise error)
try:
    immutable_global.set_value(store, wasmtime.Val.f64(2.71828))
except wasmtime.WasmtimeError as e:
    print(f"Expected error for immutable global: {e}")

# Check global types
mutable_type = mutable_global.type(store)
immutable_type = immutable_global.type(store)

print(f"Mutable global type: {mutable_type.content}, mutable: {mutable_type.mutability}")
print(f"Immutable global type: {immutable_type.content}, mutable: {immutable_type.mutability}")
```

### Utility Functions for Development

```python
import wasmtime
import time
import json
from typing import Dict, Any, List

class WasmtimeDebugUtils:
    """Collection of utility functions for WebAssembly development and debugging"""
    
    @staticmethod
    def module_info(module: wasmtime.Module) -> Dict[str, Any]:
        """Extract comprehensive information about a WebAssembly module"""
        info = {
            "imports": [],
            "exports": [],
            "custom_sections": {}
        }
        
        # Analyze imports
        for imp in module.imports:
            info["imports"].append({
                "module": imp.module,
                "name": imp.name,
                "type": str(imp.type),
                "type_class": type(imp.type).__name__
            })
        
        # Analyze exports
        for exp in module.exports:
            info["exports"].append({
                "name": exp.name,
                "type": str(exp.type),
                "type_class": type(exp.type).__name__
            })
        
        # Check for common custom sections
        common_sections = ["name", "producers", "target_features", "linking"]
        for section_name in common_sections:
            sections = module.custom_sections(section_name)
            if sections:
                info["custom_sections"][section_name] = f"{len(sections)} section(s)"
        
        return info
    
    @staticmethod
    def benchmark_function(func: wasmtime.Func, store: wasmtime.Store, 
                          args: List[Any], iterations: int = 1000) -> Dict[str, float]:
        """Benchmark a WebAssembly function"""
        print(f"Benchmarking function with {iterations} iterations...")
        
        # Warmup
        for _ in range(10):
            func(store, *args)
        
        # Actual benchmark
        start_time = time.perf_counter()
        for _ in range(iterations):
            result = func(store, *args)
        end_time = time.perf_counter()
        
        total_time = end_time - start_time
        avg_time = total_time / iterations
        
        return {
            "total_time_seconds": total_time,
            "average_time_microseconds": avg_time * 1_000_000,
            "calls_per_second": iterations / total_time,
            "iterations": iterations,
            "last_result": result
        }
    
    @staticmethod
    def create_test_wat(function_name: str, operation: str) -> str:
        """Generate test WAT code for common operations"""
        templates = {
            "add": f'''
            (module
              (func (export "{function_name}") (param i32 i32) (result i32)
                local.get 0
                local.get 1
                i32.add))
            ''',
            
            "factorial": f'''
            (module
              (func (export "{function_name}") (param i32) (result i32)
                (local i32)
                i32.const 1
                local.set 1
                (loop
                  local.get 0
                  i32.const 1
                  i32.le_s
                  if
                    local.get 1
                    return
                  end
                  local.get 1
                  local.get 0
                  i32.mul
                  local.set 1
                  local.get 0
                  i32.const 1
                  i32.sub
                  local.set 0
                  br 0)))
            ''',
            
            "memory_test": f'''
            (module
              (memory (export "memory") 1)
              (func (export "{function_name}") (param i32 i32)
                local.get 0
                local.get 1
                i32.store))
            '''
        }
        
        return templates.get(operation, templates["add"])
    
    @staticmethod
    def validate_and_analyze_wat(wat_code: str) -> Dict[str, Any]:
        """Validate WAT code and provide analysis"""
        try:
            # Convert WAT to WASM
            wasm_bytes = wasmtime.wat2wasm(wat_code)
            
            # Create module for analysis
            engine = wasmtime.Engine()
            module = wasmtime.Module(engine, wasm_bytes)
            
            analysis = {
                "valid": True,
                "wasm_size": len(wasm_bytes),
                "module_info": WasmtimeDebugUtils.module_info(module),
                "error": None
            }
            
        except wasmtime.WasmtimeError as e:
            analysis = {
                "valid": False,
                "wasm_size": 0,
                "module_info": None,
                "error": str(e)
            }
        
        return analysis

# Example usage of utility functions
if __name__ == "__main__":
    utils = WasmtimeDebugUtils()
    
    # Test WAT validation
    test_wat = utils.create_test_wat("test_add", "add")
    analysis = utils.validate_and_analyze_wat(test_wat)
    
    print("WAT Analysis:")
    print(json.dumps(analysis, indent=2))
    
    if analysis["valid"]:
        # Create and benchmark the function
        engine = wasmtime.Engine()
        store = wasmtime.Store(engine)
        wasm_bytes = wasmtime.wat2wasm(test_wat)
        module = wasmtime.Module(engine, wasm_bytes)
        instance = wasmtime.Instance(store, module, [])
        
        add_func = instance.exports(store)["test_add"]
        benchmark = utils.benchmark_function(add_func, store, [100, 200], 10000)
        
        print("\nBenchmark Results:")
        print(json.dumps(benchmark, indent=2))
    
    # Test factorial function
    factorial_wat = utils.create_test_wat("factorial", "factorial")
    factorial_analysis = utils.validate_and_analyze_wat(factorial_wat)
    
    if factorial_analysis["valid"]:
        engine = wasmtime.Engine()
        store = wasmtime.Store(engine)
        wasm_bytes = wasmtime.wat2wasm(factorial_wat)
        module = wasmtime.Module(engine, wasm_bytes)
        instance = wasmtime.Instance(store, module, [])
        
        factorial_func = instance.exports(store)["factorial"]
        
        # Test factorial computation
        for n in range(1, 8):
            result = factorial_func(store, n)
            print(f"{n}! = {result}")
```