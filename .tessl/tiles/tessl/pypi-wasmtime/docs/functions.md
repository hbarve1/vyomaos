# Function Invocation

Function calling interface supporting both Python-defined host functions and WebAssembly-exported functions. Provides comprehensive parameter marshalling, result handling, caller context access, and instance management for seamless interoperability between Python and WebAssembly code.

## Capabilities

### Function Objects

WebAssembly function wrapper providing type-safe function invocation, parameter validation, and result handling for both host-defined and WebAssembly-exported functions.

```python { .api }
class Func:
    def __init__(self, store: Store, ty: FuncType, func: Callable, access_caller: bool = False):
        """
        Create a WebAssembly function from Python callable.
        
        Parameters:
        - store: Store for function execution context
        - ty: Function type signature
        - func: Python callable implementing the function
        - access_caller: Whether function needs caller context access
        """
    
    def type(self, store: Store) -> FuncType:
        """
        Get the function type signature.
        
        Parameters:
        - store: Store context
        
        Returns:
        Function type with parameter and result types
        """
    
    def param_arity(self) -> int:
        """
        Get the number of parameters.
        
        Returns:
        Number of function parameters
        """
    
    def result_arity(self) -> int:
        """
        Get the number of results.
        
        Returns:
        Number of function return values
        """
    
    def call(self, store: Store, *args) -> Union[Val, List[Val], None]:
        """
        Call the function with given arguments.
        
        Parameters:
        - store: Store execution context
        - *args: Function arguments (Python values or Val objects)
        
        Returns:
        - None for functions with no results
        - Single Val for functions with one result
        - List[Val] for functions with multiple results
        
        Raises:
        Trap: If function execution traps
        WasmtimeError: If arguments don't match function signature
        """
```

### Caller Context

Caller context providing access to the calling instance's exports during host function execution, enabling host functions to interact with WebAssembly module state.

```python { .api }
class Caller:
    def get_export(self, name: str):
        """
        Get an export from the calling instance.
        
        Parameters:
        - name: Name of the export to retrieve
        
        Returns:
        Export object (Func, Memory, Table, or Global)
        
        Raises:
        WasmtimeError: If export doesn't exist
        """
```

### Instance Management

WebAssembly module instances providing access to instantiated modules, their exports, and runtime state. Instances represent live WebAssembly modules with allocated memory and callable functions.

```python { .api }
class Instance:
    def __init__(self, store: Store, module: Module, imports: List):
        """
        Instantiate a WebAssembly module.
        
        Parameters:
        - store: Store for instance execution context
        - module: Compiled WebAssembly module
        - imports: List of import objects matching module's import requirements
        
        Raises:
        WasmtimeError: If instantiation fails or imports don't match requirements
        """
    
    def exports(self, store: Store):
        """
        Get all exports as a mapping.
        
        Parameters:
        - store: Store context
        
        Returns:
        Dictionary-like object mapping export names to export objects
        """
    
    def get_export(self, store: Store, name: str):
        """
        Get a specific export by name.
        
        Parameters:
        - store: Store context
        - name: Export name to retrieve
        
        Returns:
        Export object (Func, Memory, Table, or Global)
        
        Raises:
        WasmtimeError: If export doesn't exist
        """
```

## Usage Examples

### Calling WebAssembly Functions

```python
import wasmtime

# Setup engine, store, and module
engine = wasmtime.Engine()
store = wasmtime.Store(engine)
wasm_bytes = wasmtime.wat2wasm('''
  (module
    (func (export "add") (param i32 i32) (result i32)
      local.get 0
      local.get 1  
      i32.add)
    (func (export "multiply_add") (param f64 f64 f64) (result f64)
      local.get 0
      local.get 1
      f64.mul
      local.get 2
      f64.add)
  )
''')

module = wasmtime.Module(engine, wasm_bytes)
instance = wasmtime.Instance(store, module, [])

# Get exported functions
exports = instance.exports(store)
add_func = exports["add"]
multiply_add_func = exports["multiply_add"]

# Call functions with Python values (automatic conversion)
result = add_func(store, 5, 3)
print(f"5 + 3 = {result}")  # Output: 5 + 3 = 8

# Call with explicit Val objects
result = multiply_add_func(store, 2.5, 3.0, 1.0)
print(f"2.5 * 3.0 + 1.0 = {result}")  # Output: 2.5 * 3.0 + 1.0 = 8.5

# Check function signatures
add_type = add_func.type(store)
print(f"Add function: {len(add_type.params)} params -> {len(add_type.results)} results")
```

### Creating Host Functions

```python
import wasmtime

# Define host function
def host_log(caller: wasmtime.Caller, message_ptr: int, message_len: int) -> None:
    """Host function that logs messages from WebAssembly"""
    # Get memory from caller
    memory = caller.get_export("memory")
    
    # Read string from WebAssembly memory
    message_bytes = memory.read(caller, message_ptr, message_ptr + message_len)
    message = message_bytes.decode('utf-8')
    print(f"[WASM LOG]: {message}")

# Create function type: (i32, i32) -> ()
log_type = wasmtime.FuncType(
    [wasmtime.ValType.I32, wasmtime.ValType.I32],
    []
)

# Create host function with caller access
engine = wasmtime.Engine()
store = wasmtime.Store(engine)
log_func = wasmtime.Func(store, log_type, host_log, access_caller=True)

# Use in module instantiation
imports = [log_func]
# ... instantiate module with imports
```

### Multiple Return Values

```python
import wasmtime

# WebAssembly function returning multiple values
wasm_bytes = wasmtime.wat2wasm('''
  (module
    (func (export "divmod") (param i32 i32) (result i32 i32)
      local.get 0
      local.get 1
      i32.div_s
      local.get 0
      local.get 1
      i32.rem_s)
  )
''')

engine = wasmtime.Engine()
store = wasmtime.Store(engine)
module = wasmtime.Module(engine, wasm_bytes)
instance = wasmtime.Instance(store, module, [])

# Call function returning multiple values
divmod_func = instance.exports(store)["divmod"]
results = divmod_func(store, 17, 5)

print(f"17 divmod 5 = {results}")  # List of Val objects
print(f"Quotient: {results[0].value}, Remainder: {results[1].value}")
```

### Host Function with Complex Logic

```python
import wasmtime
import json

def json_parse_host(caller: wasmtime.Caller, json_ptr: int, json_len: int, result_ptr: int) -> int:
    """
    Host function that parses JSON and writes result to WebAssembly memory.
    Returns 1 on success, 0 on error.
    """
    try:
        # Get memory from caller instance
        memory = caller.get_export("memory")
        
        # Read JSON string from WebAssembly memory
        json_bytes = memory.read(caller, json_ptr, json_ptr + json_len)
        json_str = json_bytes.decode('utf-8')
        
        # Parse JSON
        parsed = json.loads(json_str)
        
        # For simplicity, just write the number of keys back
        if isinstance(parsed, dict):
            key_count = len(parsed)
            # Write 4-byte integer to result_ptr
            memory.write(caller, key_count.to_bytes(4, 'little'), result_ptr)
            return 1
        else:
            return 0
    except Exception as e:
        print(f"JSON parse error: {e}")
        return 0

# Create function type: (i32, i32, i32) -> i32
json_parse_type = wasmtime.FuncType(
    [wasmtime.ValType.I32, wasmtime.ValType.I32, wasmtime.ValType.I32],
    [wasmtime.ValType.I32]
)

# Create host function
engine = wasmtime.Engine()
store = wasmtime.Store(engine)
json_parse_func = wasmtime.Func(store, json_parse_type, json_parse_host, access_caller=True)
```

### Function Introspection

```python
import wasmtime

# Load and inspect a WebAssembly module
engine = wasmtime.Engine()
module = wasmtime.Module.from_file(engine, "complex_module.wasm")

# Examine exported functions
for export in module.exports:
    if isinstance(export.type, wasmtime.FuncType):
        func_type = export.type
        print(f"Function {export.name}:")
        print(f"  Parameters: {len(func_type.params)}")
        for i, param_type in enumerate(func_type.params):
            print(f"    param[{i}]: {param_type}")
        print(f"  Results: {len(func_type.results)}")
        for i, result_type in enumerate(func_type.results):
            print(f"    result[{i}]: {result_type}")

# After instantiation, check function arities
store = wasmtime.Store(engine)
instance = wasmtime.Instance(store, module, imports)

for name in ["add", "process", "calculate"]:
    try:
        func = instance.get_export(store, name)
        if isinstance(func, wasmtime.Func):
            print(f"{name}: {func.param_arity()} params -> {func.result_arity()} results")
    except wasmtime.WasmtimeError:
        print(f"Function {name} not found")
```

### Error Handling in Function Calls

```python
import wasmtime

def safe_divide_host(x: int, y: int) -> int:
    """Host function that safely divides two numbers"""
    if y == 0:
        # Raise trap for division by zero
        raise wasmtime.Trap("Division by zero")
    return x // y

# Create function type and host function
divide_type = wasmtime.FuncType(
    [wasmtime.ValType.I32, wasmtime.ValType.I32],
    [wasmtime.ValType.I32]
)

engine = wasmtime.Engine()
store = wasmtime.Store(engine)
divide_func = wasmtime.Func(store, divide_type, safe_divide_host)

# Call function with error handling
try:
    result = divide_func(store, 10, 2)
    print(f"10 / 2 = {result}")  # Success
    
    result = divide_func(store, 10, 0)  # This will trap
except wasmtime.Trap as trap:
    print(f"Function trapped: {trap.message}")
except wasmtime.WasmtimeError as error:
    print(f"Runtime error: {error}")
```