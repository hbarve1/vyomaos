# Wasmtime

A comprehensive Python library providing high-performance WebAssembly execution through the Wasmtime runtime. This package offers complete Python bindings for WebAssembly module compilation, instantiation, and execution, along with full WASI support, component model integration, and advanced runtime features for serverless computing, plugin systems, and cross-language interoperability.

## Package Information

- **Package Name**: wasmtime
- **Language**: Python  
- **Installation**: `pip install wasmtime`
- **Minimum Python**: 3.9+

## Core Imports

```python
import wasmtime
```

Import individual components:

```python
from wasmtime import Engine, Store, Module, Instance
from wasmtime import wat2wasm, WasiConfig, Linker
```

## Basic Usage

```python
import wasmtime

# Create a WebAssembly engine and store
engine = wasmtime.Engine()
store = wasmtime.Store(engine)

# Compile WebAssembly from WAT (WebAssembly Text format)
wasm_bytes = wasmtime.wat2wasm('''
  (module
    (func (export "add") (param i32 i32) (result i32)
      local.get 0
      local.get 1
      i32.add)
  )
''')

# Compile the module
module = wasmtime.Module(engine, wasm_bytes)

# Instantiate the module
instance = wasmtime.Instance(store, module, [])

# Call the exported function
add_func = instance.exports(store)["add"]
result = add_func(store, 5, 3)
print(f"5 + 3 = {result}")  # Output: 5 + 3 = 8

# Using WASI for system interface
wasi_config = wasmtime.WasiConfig()
wasi_config.inherit_argv()
wasi_config.inherit_env()
wasi_config.inherit_stdin()
wasi_config.inherit_stdout()
wasi_config.inherit_stderr()

# Create linker with WASI support
linker = wasmtime.Linker(engine)
linker.define_wasi(store, wasi_config)
```

## Architecture

Wasmtime's Python API follows a hierarchical structure reflecting WebAssembly's runtime model:

- **Engine**: Top-level compilation engine managing WebAssembly compilation settings and optimizations
- **Store**: Runtime state container holding instances, memories, globals, and execution context
- **Module**: Compiled WebAssembly module containing validated and optimized code
- **Instance**: Live instantiation of a module with allocated memory, globals, and callable functions
- **Linker**: Module linking system for resolving imports and connecting multiple modules

This design enables efficient WebAssembly execution while providing fine-grained control over compilation, memory management, security boundaries, and system interface integration through WASI.

## Capabilities

### Core Runtime

WebAssembly engine configuration, module compilation, and execution environment management. Provides the foundation for all WebAssembly operations including compilation strategies, debugging support, and performance optimizations.

```python { .api }
class Engine:
    def __init__(self, config: Config = None): ...
    def increment_epoch(self) -> None: ...
    def is_pulley(self) -> bool: ...

class Store:
    def __init__(self, engine: Engine = None, data: Any = None): ...
    def add_fuel(self, fuel: int) -> None: ...
    def fuel_consumed(self) -> int: ...
    def set_epoch_deadline(self, ticks: int) -> None: ...

class Module:
    def __init__(self, engine: Engine, wasm: bytes, validate: bool = True): ...
    @classmethod
    def from_file(cls, engine: Engine, path: str, validate: bool = True) -> 'Module': ...
    @staticmethod  
    def validate(engine: Engine, wasm: bytes) -> None: ...
    def serialize(self) -> bytes: ...
    @classmethod
    def deserialize(cls, engine: Engine, encoded: bytes) -> 'Module': ...
```

[Core Runtime](./core-runtime.md)

### WebAssembly Types

Complete type system mapping WebAssembly value types, function signatures, memory layouts, and table definitions to Python. Includes comprehensive type validation and conversion utilities.

```python { .api }
class ValType:
    I32: 'ValType'
    I64: 'ValType' 
    F32: 'ValType'
    F64: 'ValType'
    V128: 'ValType'
    FUNCREF: 'ValType'
    EXTERNREF: 'ValType'

class FuncType:
    def __init__(self, params: List[ValType], results: List[ValType]): ...
    @property
    def params(self) -> List[ValType]: ...
    @property
    def results(self) -> List[ValType]: ...

class Val:
    @staticmethod
    def i32(val: int) -> 'Val': ...
    @staticmethod
    def i64(val: int) -> 'Val': ...
    @staticmethod
    def f32(val: float) -> 'Val': ...
    @staticmethod
    def f64(val: float) -> 'Val': ...
```

[WebAssembly Types](./types.md)

### Function Invocation

Function calling interface supporting both Python-defined and WebAssembly-exported functions. Includes parameter marshalling, result handling, and caller context access for host function implementations.

```python { .api }
class Func:
    def __init__(self, store: Store, ty: FuncType, func: Callable, access_caller: bool = False): ...
    def type(self, store: Store) -> FuncType: ...
    def param_arity(self) -> int: ...
    def result_arity(self) -> int: ...
    def call(self, store: Store, *args) -> Union[Val, List[Val], None]: ...

class Caller:
    def get_export(self, name: str): ...
    
class Instance:
    def __init__(self, store: Store, module: Module, imports: List): ...
    def exports(self, store: Store): ...
    def get_export(self, store: Store, name: str): ...
```

[Function Invocation](./functions.md)

### Memory Management

Linear memory allocation, access, and manipulation supporting both regular and shared memory models. Includes memory growth, data transfer, and multi-threading coordination.

```python { .api }
class Memory:
    def __init__(self, store: Store, ty: MemoryType): ...
    def type(self, store: Store) -> MemoryType: ...
    def grow(self, store: Store, delta: int) -> int: ...
    def size(self, store: Store) -> int: ...
    def data_len(self, store: Store) -> int: ...
    def data_ptr(self, store: Store) -> int: ...
    def read(self, store: Store, start: int, stop: int) -> bytes: ...
    def write(self, store: Store, data: bytes, start: int = 0) -> None: ...

class SharedMemory:
    def __init__(self, engine: Engine, ty: MemoryType): ...
    def type(self) -> MemoryType: ...
    def as_memory(self, store: Store) -> Memory: ...
```

[Memory Management](./memory.md)

### WASI Integration

Complete WebAssembly System Interface implementation providing filesystem access, environment variables, command-line arguments, and I/O redirection with comprehensive permission controls.

```python { .api }
class WasiConfig:
    def __init__(self): ...
    def argv(self, argv: List[str]) -> None: ...
    def inherit_argv(self) -> None: ...
    def env(self, env: List[Tuple[str, str]]) -> None: ...
    def inherit_env(self) -> None: ...
    def stdin_file(self, path: str) -> None: ...
    def stdout_file(self, path: str) -> None: ...
    def stderr_file(self, path: str) -> None: ...
    def inherit_stdin(self) -> None: ...
    def inherit_stdout(self) -> None: ...
    def inherit_stderr(self) -> None: ...
    def preopen_dir(self, path: str, guest_path: str, perms: DirPerms) -> None: ...

class DirPerms:
    READ_ONLY: 'DirPerms'
    WRITE_ONLY: 'DirPerms' 
    READ_WRITE: 'DirPerms'

class FilePerms:
    READ_ONLY: 'FilePerms'
    WRITE_ONLY: 'FilePerms'
    READ_WRITE: 'FilePerms'
```

[WASI Integration](./wasi.md)

### Module Linking

Advanced module linking system for resolving imports, connecting multiple WebAssembly modules, and creating complex WebAssembly applications with shared functionality.

```python { .api }
class Linker:
    def __init__(self, engine: Engine): ...
    def allow_shadowing(self, enable: bool) -> None: ...
    def define(self, store: Store, module: str, name: str, item) -> None: ...
    def define_func(self, module: str, name: str, ty: FuncType, func: Callable) -> None: ...
    def define_wasi(self, store: Store, wasi_config: WasiConfig) -> None: ...
    def define_instance(self, store: Store, name: str, instance: Instance) -> None: ...
    def instantiate(self, store: Store, module: Module) -> Instance: ...
    def get_default(self, store: Store, name: str): ...
```

[Module Linking](./linking.md)

### Error Handling

Comprehensive error handling system including WebAssembly traps, stack traces, runtime errors, and WASI exit codes with detailed debugging information.

```python { .api }
class WasmtimeError(Exception): ...

class Trap(Exception):
    def __init__(self, message: str): ...
    @property
    def message(self) -> str: ...
    @property  
    def frames(self) -> List[Frame]: ...
    @property
    def trace(self) -> List[str]: ...

class Frame:
    @property
    def func_name(self) -> Optional[str]: ...
    @property
    def module_name(self) -> Optional[str]: ...
    @property
    def func_index(self) -> int: ...
    @property
    def module_offset(self) -> int: ...

class ExitTrap(Exception):
    def __init__(self, code: int): ...
    @property
    def code(self) -> int: ...
```

[Error Handling](./errors.md)

### Component Model

WebAssembly component model integration through bindgen tooling, enabling rich type communication between Python and WebAssembly components with automatic binding generation.

```python { .api }
import wasmtime.bindgen

def generate(name: str, component: bytes) -> Dict[str, bytes]: ...
```

[Component Model](./components.md)

### Utilities

Helper functions and utilities including WebAssembly text format conversion, table management, global variable handling, and development aids.

```python { .api }
def wat2wasm(wat: str) -> bytes: ...

class Table:
    def __init__(self, store: Store, ty: TableType, init): ...
    def type(self, store: Store) -> TableType: ...
    def size(self, store: Store) -> int: ...
    def grow(self, store: Store, delta: int, init) -> int: ...
    def get(self, store: Store, idx: int): ...
    def set(self, store: Store, idx: int, val) -> None: ...

class Global:
    def __init__(self, store: Store, ty: GlobalType, val: Val): ...
    def type(self, store: Store) -> GlobalType: ...
    def value(self, store: Store) -> Val: ...
    def set_value(self, store: Store, val: Val) -> None: ...
```

[Utilities](./utilities.md)