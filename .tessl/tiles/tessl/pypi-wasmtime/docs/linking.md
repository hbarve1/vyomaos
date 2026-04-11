# Module Linking

Advanced module linking system for resolving imports, connecting multiple WebAssembly modules, and creating complex WebAssembly applications with shared functionality, WASI integration, and dynamic module loading.

## Capabilities

### Linker Management

Central linking system managing module imports, instance creation, and cross-module communication with support for shadowing, WASI integration, and complex dependency resolution.

```python { .api }
class Linker:
    def __init__(self, engine: Engine):
        """
        Create a new linker for the given engine.
        
        Parameters:
        - engine: WebAssembly engine for linking context
        """
    
    def allow_shadowing(self, enable: bool) -> None:
        """
        Configure whether export shadowing is allowed.
        
        Parameters:
        - enable: True to allow shadowing, False to error on conflicts
        """
    
    def define(self, store: Store, module: str, name: str, item) -> None:
        """
        Define an import with a specific module and name.
        
        Parameters:
        - store: Store context
        - module: Module name for the import
        - name: Import name within the module
        - item: Export object (Func, Memory, Table, or Global)
        """
    
    def define_func(self, module: str, name: str, ty: FuncType, func: Callable) -> None:
        """
        Define a function import directly from Python callable.
        
        Parameters:
        - module: Module name for the import
        - name: Function name within the module
        - ty: Function type signature
        - func: Python callable implementing the function
        """
    
    def define_wasi(self, store: Store, wasi_config: WasiConfig) -> None:
        """
        Define WASI imports using the given configuration.
        
        Parameters:
        - store: Store context
        - wasi_config: WASI configuration for system interface
        """
    
    def define_instance(self, store: Store, name: str, instance: Instance) -> None:
        """
        Define all exports from an instance as imports.
        
        Parameters:
        - store: Store context
        - name: Module name for the instance exports
        - instance: WebAssembly instance to export
        """
    
    def instantiate(self, store: Store, module: Module) -> Instance:
        """
        Instantiate a module using defined imports.
        
        Parameters:
        - store: Store context
        - module: WebAssembly module to instantiate
        
        Returns:
        Instantiated WebAssembly instance
        
        Raises:
        WasmtimeError: If imports don't satisfy module requirements
        """
    
    def get_default(self, store: Store, name: str):
        """
        Get the default export for a module.
        
        Parameters:
        - store: Store context
        - name: Module name
        
        Returns:
        Default export object if available
        """
```

## Usage Examples

### Basic Module Linking

```python
import wasmtime

# Create engine and linker
engine = wasmtime.Engine()
linker = wasmtime.Linker(engine)
store = wasmtime.Store(engine)

# Define host functions for import
def host_print(message_ptr: int, message_len: int) -> None:
    # Implementation would read from memory and print
    print(f"Host print called with ptr={message_ptr}, len={message_len}")

print_type = wasmtime.FuncType([wasmtime.ValType.I32, wasmtime.ValType.I32], [])
linker.define_func("host", "print", print_type, host_print)

# Define memory for sharing between modules
memory_type = wasmtime.MemoryType(wasmtime.Limits(1, 10))
shared_memory = wasmtime.Memory(store, memory_type)
linker.define(store, "env", "memory", shared_memory)

# Load and instantiate module
module = wasmtime.Module.from_file(engine, "program.wasm")
instance = linker.instantiate(store, module)

print("Module instantiated with host imports")
```

### Multi-Module Application

```python
import wasmtime

engine = wasmtime.Engine()
linker = wasmtime.Linker(engine)
store = wasmtime.Store(engine)

# Load utility module first
utils_module = wasmtime.Module.from_file(engine, "utils.wasm")
utils_instance = linker.instantiate(store, utils_module)

# Make utils exports available to other modules
linker.define_instance(store, "utils", utils_instance)

# Load main application module that imports from utils
app_module = wasmtime.Module.from_file(engine, "app.wasm")
app_instance = linker.instantiate(store, app_module)

# Now app can use functions from utils module
main_func = app_instance.get_export(store, "main")
result = main_func(store)
print(f"Application result: {result}")
```

### WASI Application with Custom Imports

```python
import wasmtime
import time

# Custom host functions for the application
def get_timestamp() -> int:
    """Return current Unix timestamp"""
    return int(time.time())

def sleep_ms(milliseconds: int) -> None:
    """Sleep for specified milliseconds"""
    time.sleep(milliseconds / 1000.0)

# Set up linker with custom functions
engine = wasmtime.Engine()
linker = wasmtime.Linker(engine)
store = wasmtime.Store(engine)

# Define custom host functions
timestamp_type = wasmtime.FuncType([], [wasmtime.ValType.I32])
sleep_type = wasmtime.FuncType([wasmtime.ValType.I32], [])

linker.define_func("host", "get_timestamp", timestamp_type, get_timestamp)
linker.define_func("host", "sleep_ms", sleep_type, sleep_ms)

# Configure and define WASI
wasi_config = wasmtime.WasiConfig()
wasi_config.inherit_argv()
wasi_config.inherit_env()
wasi_config.inherit_stdin()
wasi_config.inherit_stdout()
wasi_config.inherit_stderr()
linker.define_wasi(store, wasi_config)

# Load and run WASI application with custom imports
module = wasmtime.Module.from_file(engine, "wasi_app.wasm")
instance = linker.instantiate(store, module)

# Start the application
try:
    start_func = instance.get_export(store, "_start")
    start_func(store)
except wasmtime.ExitTrap as exit_trap:
    print(f"Application exited with code: {exit_trap.code}")
```

### Dynamic Module Loading

```python
import wasmtime
import os

class ModuleLoader:
    def __init__(self, engine: wasmtime.Engine):
        self.engine = engine
        self.linker = wasmtime.Linker(engine)
        self.store = wasmtime.Store(engine)
        self.loaded_modules = {}
        
        # Allow shadowing for dynamic loading
        self.linker.allow_shadowing(True)
        
        # Set up WASI by default
        wasi_config = wasmtime.WasiConfig()
        wasi_config.inherit_argv()
        wasi_config.inherit_env()
        wasi_config.inherit_stdin()
        wasi_config.inherit_stdout()
        wasi_config.inherit_stderr()
        self.linker.define_wasi(self.store, wasi_config)
    
    def load_module(self, name: str, path: str) -> wasmtime.Instance:
        """Load a WebAssembly module and make its exports available"""
        if name in self.loaded_modules:
            return self.loaded_modules[name]
        
        # Load and instantiate module
        module = wasmtime.Module.from_file(self.engine, path)
        instance = self.linker.instantiate(self.store, module)
        
        # Make this module's exports available to future modules
        self.linker.define_instance(self.store, name, instance)
        
        # Cache the instance
        self.loaded_modules[name] = instance
        
        print(f"Loaded module '{name}' from {path}")
        return instance
    
    def get_module(self, name: str) -> wasmtime.Instance:
        """Get a previously loaded module"""
        if name not in self.loaded_modules:
            raise ValueError(f"Module '{name}' not loaded")
        return self.loaded_modules[name]

# Usage example
engine = wasmtime.Engine()
loader = ModuleLoader(engine)

# Load modules in dependency order
math_instance = loader.load_module("math", "math_utils.wasm")
string_instance = loader.load_module("string", "string_utils.wasm")
app_instance = loader.load_module("app", "main_app.wasm")

# The main app can now use functions from both math and string modules
```

### Import Resolution with Error Handling

```python
import wasmtime

def safe_module_linking(engine: wasmtime.Engine, module_path: str, imports_config: dict):
    """
    Safely link and instantiate a module with comprehensive error handling.
    
    Parameters:
    - engine: WebAssembly engine
    - module_path: Path to WebAssembly module
    - imports_config: Dictionary of import configurations
    """
    
    linker = wasmtime.Linker(engine)
    store = wasmtime.Store(engine)
    
    try:
        # Load the module first to check its imports
        module = wasmtime.Module.from_file(engine, module_path)
        
        print(f"Module requires {len(module.imports)} imports:")
        for import_item in module.imports:
            print(f"  {import_item.module}.{import_item.name}: {import_item.type}")
        
        # Define imports based on configuration
        for import_item in module.imports:
            module_name = import_item.module
            item_name = import_item.name
            
            if module_name in imports_config:
                if item_name in imports_config[module_name]:
                    import_obj = imports_config[module_name][item_name]
                    linker.define(store, module_name, item_name, import_obj)
                    print(f"  ✓ Resolved {module_name}.{item_name}")
                else:
                    print(f"  ✗ Missing import: {module_name}.{item_name}")
            else:
                print(f"  ✗ Missing module: {module_name}")
        
        # Try to instantiate
        print("Attempting instantiation...")
        instance = linker.instantiate(store, module)
        print("✓ Module instantiated successfully")
        
        return instance
        
    except wasmtime.WasmtimeError as e:
        if "unknown import" in str(e):
            print(f"✗ Import resolution failed: {e}")
            print("Available imports in linker:")
            # Note: Wasmtime doesn't provide a way to list available imports
            print("  (Check your imports_config)")
        else:
            print(f"✗ Linking error: {e}")
        return None
        
    except FileNotFoundError:
        print(f"✗ Module file not found: {module_path}")
        return None
        
    except Exception as e:
        print(f"✗ Unexpected error: {e}")
        return None

# Example usage
engine = wasmtime.Engine()
store = wasmtime.Store(engine)

# Create some sample imports
memory = wasmtime.Memory(store, wasmtime.MemoryType(wasmtime.Limits(1)))
print_func = wasmtime.Func(store, 
    wasmtime.FuncType([wasmtime.ValType.I32], []),
    lambda x: print(f"Printed: {x}"))

imports_config = {
    "env": {
        "memory": memory,
        "print": print_func
    }
}

instance = safe_module_linking(engine, "test_module.wasm", imports_config)
if instance:
    # Use the instance...
    pass
```