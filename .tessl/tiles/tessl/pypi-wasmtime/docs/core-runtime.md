# Core Runtime

The core runtime provides the foundational components for WebAssembly execution including engine configuration, module compilation, instance management, and execution environment control. These components establish the foundation for all WebAssembly operations.

## Capabilities

### Engine Configuration

Global configuration system controlling WebAssembly compilation strategy, optimization level, debugging features, and runtime behavior. Configuration must be set before engine creation and cannot be modified afterwards.

```python { .api }
class Config:
    def __init__(self): ...
    
    # Compilation and optimization settings
    def debug_info(self, enable: bool) -> None:
        """Enable DWARF debug information generation for profiling and debugging"""
    
    def strategy(self, strategy: str) -> None:
        """Set compilation strategy: 'auto', 'cranelift'"""
    
    def profiler(self, profiler: str) -> None:
        """Enable profiler: 'none', 'jitdump', 'vtune'"""
    
    # WebAssembly proposals
    def wasm_threads(self, enable: bool) -> None:
        """Enable WebAssembly threads proposal"""
    
    def wasm_reference_types(self, enable: bool) -> None:
        """Enable WebAssembly reference types proposal"""
    
    def wasm_simd(self, enable: bool) -> None:
        """Enable WebAssembly SIMD proposal"""
    
    def wasm_bulk_memory(self, enable: bool) -> None:
        """Enable WebAssembly bulk memory operations proposal"""
    
    def wasm_multi_value(self, enable: bool) -> None:
        """Enable WebAssembly multi-value proposal"""
    
    def wasm_tail_call(self, enable: bool) -> None:
        """Enable WebAssembly tail call proposal"""
    
    def wasm_multi_memory(self, enable: bool) -> None:
        """Enable WebAssembly multi-memory proposal"""
    
    def wasm_memory64(self, enable: bool) -> None:
        """Enable WebAssembly memory64 proposal"""
    
    def wasm_relaxed_simd(self, enable: bool) -> None:
        """Enable WebAssembly relaxed SIMD proposal"""
    
    def wasm_relaxed_simd_deterministic(self, enable: bool) -> None:
        """Enable deterministic mode for relaxed SIMD proposal"""
    
    # Compilation and optimization controls
    def cranelift_debug_verifier(self, enable: bool) -> None:
        """Enable Cranelift debug verifier"""
    
    def cranelift_opt_level(self, opt_level: str) -> None:
        """Set Cranelift optimization level: 'none', 'speed', 'speed_and_size'"""
    
    # Performance and execution controls
    def consume_fuel(self, enable: bool) -> None:
        """Enable fuel-based execution limiting"""
    
    def epoch_interruption(self, enable: bool) -> None:
        """Enable epoch-based interruption for long-running code"""
    
    def parallel_compilation(self, enable: bool) -> None:
        """Enable parallel compilation of WebAssembly functions"""
    
    # Caching
    def cache(self, enabled: Union[bool, str]) -> None:
        """Configure compilation caching: True/False or cache directory path"""
```

### Engine Management

WebAssembly compilation engine responsible for compiling modules, managing compilation caches, and providing runtime services. The engine is the top-level context for all WebAssembly operations.

```python { .api }
class Engine:
    def __init__(self, config: Config = None):
        """
        Create a new WebAssembly engine.
        
        Parameters:
        - config: Optional configuration. If None, uses default settings.
        """
    
    def increment_epoch(self) -> None:
        """
        Increment the epoch counter for epoch-based interruption.
        Used to interrupt long-running WebAssembly code.
        """
    
    def is_pulley(self) -> bool:
        """
        Check if the engine is using the Pulley backend.
        
        Returns:
        True if using Pulley backend, False otherwise.
        """
```

### Store Management

Runtime state container managing execution context, fuel consumption, epoch deadlines, and instance lifecycle. Each store provides isolated execution environment for WebAssembly instances.

```python { .api }
class Store:
    def __init__(self, engine: Engine = None, data: Any = None):
        """
        Create a new store for WebAssembly execution.
        
        Parameters:
        - engine: WebAssembly engine. If None, creates default engine.
        - data: Optional user data associated with the store.
        """
    
    def data(self) -> Any:
        """Get the user data associated with this store"""
    
    def gc(self) -> None:
        """Run garbage collection on externref values in the store"""
    
    # Fuel-based execution control
    def set_fuel(self, fuel: int) -> None:
        """
        Set the fuel available for execution in this store.
        
        Parameters:
        - fuel: Amount of fuel to set (must be non-negative)
        
        Raises:
        WasmtimeError: If fuel consumption is not enabled in configuration
        """
    
    def get_fuel(self) -> int:
        """
        Get the amount of fuel remaining in this store.
        
        Returns:
        Amount of fuel remaining
        
        Raises:
        WasmtimeError: If fuel consumption is not enabled in configuration
        """
    
    # WASI integration
    def set_wasi(self, wasi: WasiConfig) -> None:
        """
        Configure WASI for this store.
        
        Parameters:
        - wasi: WASI configuration object
        
        Raises:
        WasmtimeError: If WASI configuration fails
        """
    
    # Epoch-based interruption
    def set_epoch_deadline(self, ticks_after_current: int) -> None:
        """
        Set relative epoch deadline for interrupting long-running code.
        
        Parameters:
        - ticks_after_current: Number of epoch ticks after current epoch before interruption
        """
    
    # Resource limits
    def set_limits(self, memory_size: int = -1, table_elements: int = -1, 
                   instances: int = -1, tables: int = -1, memories: int = -1) -> None:
        """
        Configure resource limits for this store.
        
        Parameters:
        - memory_size: Maximum linear memory size in bytes (-1 for unlimited)
        - table_elements: Maximum table elements (-1 for unlimited) 
        - instances: Maximum WebAssembly instances (-1 for unlimited)
        - tables: Maximum tables (-1 for unlimited)
        - memories: Maximum linear memories (-1 for unlimited)
        """
```

### Module Compilation

WebAssembly module compilation and validation providing binary parsing, validation, serialization, and metadata extraction. Compiled modules can be instantiated multiple times and shared between stores.

```python { .api }
class Module:
    def __init__(self, engine: Engine, wasm: bytes, validate: bool = True):
        """
        Compile a WebAssembly module from binary.
        
        Parameters:
        - engine: WebAssembly engine for compilation
        - wasm: WebAssembly binary data
        - validate: Whether to validate the module (default: True)
        
        Raises:
        WasmtimeError: If compilation or validation fails
        """
    
    @classmethod
    def from_file(cls, engine: Engine, path: str, validate: bool = True) -> 'Module':
        """
        Compile a WebAssembly module from file.
        
        Parameters:
        - engine: WebAssembly engine for compilation
        - path: Path to WebAssembly binary file
        - validate: Whether to validate the module (default: True)
        
        Returns:
        Compiled WebAssembly module
        
        Raises:
        WasmtimeError: If compilation, validation, or file reading fails
        """
    
    @staticmethod
    def validate(engine: Engine, wasm: bytes) -> None:
        """
        Validate WebAssembly binary without compilation.
        
        Parameters:
        - engine: WebAssembly engine for validation context
        - wasm: WebAssembly binary data to validate
        
        Raises:
        WasmtimeError: If validation fails
        """
    
    def serialize(self) -> bytes:
        """
        Serialize compiled module for caching.
        
        Returns:
        Serialized module data
        """
    
    @classmethod
    def deserialize(cls, engine: Engine, encoded: bytes) -> 'Module':
        """
        Deserialize a previously compiled module.
        
        Parameters:
        - engine: WebAssembly engine (must be compatible)
        - encoded: Serialized module data
        
        Returns:
        Deserialized WebAssembly module
        
        Raises:
        WasmtimeError: If deserialization fails or engine incompatible
        """
    
    @property
    def imports(self) -> List[ImportType]:
        """
        Get the import requirements of this module.
        
        Returns:
        List of import type declarations
        """
    
    @property
    def exports(self) -> List[ExportType]:
        """
        Get the exports provided by this module.
        
        Returns:
        List of export type declarations
        """
    
    def custom_sections(self, name: str) -> List[bytes]:
        """
        Get custom sections with the specified name.
        
        Parameters:
        - name: Name of custom section to retrieve
        
        Returns:
        List of custom section data
        """
```

## Usage Examples

### Basic Engine and Store Setup

```python
import wasmtime

# Create configuration with debugging enabled
config = wasmtime.Config()
config.debug_info(True)
config.wasm_simd(True)
config.consume_fuel(True)

# Create engine with configuration
engine = wasmtime.Engine(config)

# Create store with custom data
user_data = {"request_id": "123", "user": "alice"}
store = wasmtime.Store(engine, user_data)

# Set fuel for execution limiting
store.set_fuel(1000000)
```

### Module Compilation and Caching

```python
import wasmtime

engine = wasmtime.Engine()

# Compile from binary data
wasm_bytes = load_wasm_file("module.wasm")
module = wasmtime.Module(engine, wasm_bytes)

# Serialize for caching
cached_data = module.serialize()
save_to_cache(cached_data)

# Later: deserialize from cache
cached_data = load_from_cache()
module = wasmtime.Module.deserialize(engine, cached_data)

# Inspect module metadata
print(f"Module imports: {len(module.imports)}")
print(f"Module exports: {len(module.exports)}")
for export in module.exports:
    print(f"  Export: {export.name} ({export.type})")
```

### Fuel-based Execution Control

```python
import wasmtime

config = wasmtime.Config()
config.consume_fuel(True)
engine = wasmtime.Engine(config)
store = wasmtime.Store(engine)

# Set fuel before execution
store.set_fuel(100000)

# Execute WebAssembly code...
# func(store, args...)

# Check remaining fuel
remaining = store.get_fuel()
print(f"Fuel remaining: {remaining}")

# Set more fuel if needed
if remaining < 10000:
    store.set_fuel(100000)
```

### Epoch-based Interruption

```python
import wasmtime
import threading
import time

engine = wasmtime.Engine()
store = wasmtime.Store(engine)

# Set epoch deadline
store.set_epoch_deadline(100)

# Function to increment epoch from another thread
def epoch_incrementer():
    time.sleep(1)  # Let execution start
    for _ in range(200):
        engine.increment_epoch()
        time.sleep(0.01)

# Start epoch incrementer
thread = threading.Thread(target=epoch_incrementer)
thread.start()

# Long-running WebAssembly execution will be interrupted
# when epoch counter exceeds deadline
```