# WebAssembly Types

Complete type system mapping WebAssembly value types, function signatures, memory layouts, table definitions, and import/export declarations to Python. Provides comprehensive type validation, conversion utilities, and metadata access for WebAssembly components.

## Capabilities

### Value Types

WebAssembly value type system representing the fundamental data types supported by WebAssembly virtual machine, including numeric types, vector types, and reference types.

```python { .api }
class ValType:
    # Numeric types
    I32: 'ValType'  # 32-bit integer
    I64: 'ValType'  # 64-bit integer  
    F32: 'ValType'  # 32-bit floating point
    F64: 'ValType'  # 64-bit floating point
    
    # Vector type
    V128: 'ValType'  # 128-bit SIMD vector
    
    # Reference types
    FUNCREF: 'ValType'   # Function reference
    EXTERNREF: 'ValType' # External reference
    
    def __eq__(self, other) -> bool: ...
    def __str__(self) -> str: ...
```

### Value Wrappers

WebAssembly value wrapper providing type-safe conversion between Python values and WebAssembly runtime values, with automatic marshalling and unmarshalling support.

```python { .api }
class Val:
    @staticmethod
    def i32(val: int) -> 'Val':
        """
        Create a 32-bit integer value.
        
        Parameters:
        - val: Python integer (must fit in 32-bit signed range)
        
        Returns:
        WebAssembly i32 value
        """
    
    @staticmethod
    def i64(val: int) -> 'Val':
        """
        Create a 64-bit integer value.
        
        Parameters:
        - val: Python integer (must fit in 64-bit signed range)
        
        Returns:
        WebAssembly i64 value
        """
    
    @staticmethod
    def f32(val: float) -> 'Val':
        """
        Create a 32-bit floating point value.
        
        Parameters:
        - val: Python float
        
        Returns:
        WebAssembly f32 value
        """
    
    @staticmethod
    def f64(val: float) -> 'Val':
        """
        Create a 64-bit floating point value.
        
        Parameters:
        - val: Python float
        
        Returns:
        WebAssembly f64 value
        """
    
    @staticmethod
    def funcref(val) -> 'Val':
        """
        Create a function reference value.
        
        Parameters:
        - val: Function object or None
        
        Returns:
        WebAssembly funcref value
        """
    
    @staticmethod  
    def externref(val) -> 'Val':
        """
        Create an external reference value.
        
        Parameters:
        - val: Any Python object or None
        
        Returns:
        WebAssembly externref value
        """
    
    @property
    def value(self) -> Union[int, float, None]:
        """
        Get the raw Python value.
        
        Returns:
        The underlying Python value
        """
    
    @property
    def type(self) -> ValType:
        """
        Get the WebAssembly value type.
        
        Returns:
        The ValType of this value
        """
```

### Function Types

Function signature type defining parameter and result types for WebAssembly functions, enabling type checking and validation during function calls and imports.

```python { .api }
class FuncType:
    def __init__(self, params: List[ValType], results: List[ValType]):
        """
        Create a function type with parameter and result types.
        
        Parameters:
        - params: List of parameter value types
        - results: List of result value types
        """
    
    @property
    def params(self) -> List[ValType]:
        """
        Get the parameter types.
        
        Returns:
        List of parameter value types
        """
    
    @property
    def results(self) -> List[ValType]:
        """
        Get the result types.
        
        Returns:
        List of result value types
        """
    
    def __eq__(self, other) -> bool: ...
    def __str__(self) -> str: ...
```

### Memory Types

Memory layout specification defining size limits and growth constraints for WebAssembly linear memory, supporting both bounded and unbounded memory configurations.

```python { .api }
class MemoryType:
    def __init__(self, limits: Limits):
        """
        Create a memory type with size limits.
        
        Parameters:
        - limits: Memory size limits specification
        """
    
    @property
    def limits(self) -> Limits:
        """
        Get the memory size limits.
        
        Returns:
        Memory size limits
        """
    
    def __eq__(self, other) -> bool: ...
    def __str__(self) -> str: ...
```

### Table Types

Table type specification defining element type and size constraints for WebAssembly tables, supporting function references and external references.

```python { .api }
class TableType:
    def __init__(self, element: ValType, limits: Limits):
        """
        Create a table type with element type and size limits.
        
        Parameters:
        - element: Type of elements stored in the table
        - limits: Table size limits specification
        """
    
    @property
    def element(self) -> ValType:
        """
        Get the table element type.
        
        Returns:
        Element value type
        """
    
    @property
    def limits(self) -> Limits:
        """
        Get the table size limits.
        
        Returns:
        Table size limits
        """
    
    def __eq__(self, other) -> bool: ...
    def __str__(self) -> str: ...
```

### Global Types

Global variable type specification defining value type and mutability constraints for WebAssembly global variables, supporting both mutable and immutable globals.

```python { .api }
class GlobalType:
    def __init__(self, content: ValType, mutability: bool):
        """
        Create a global type with value type and mutability.
        
        Parameters:
        - content: Value type of the global variable
        - mutability: Whether the global is mutable
        """
    
    @property
    def content(self) -> ValType:
        """
        Get the global value type.
        
        Returns:
        Global variable value type
        """
    
    @property
    def mutability(self) -> bool:
        """
        Get the global mutability.
        
        Returns:
        True if mutable, False if immutable
        """
    
    def __eq__(self, other) -> bool: ...
    def __str__(self) -> str: ...
```

### Size Limits

Size constraint specification for memory and table types, defining minimum size requirements and optional maximum size limits with growth boundaries.

```python { .api }
class Limits:
    def __init__(self, min: int, max: Optional[int] = None):
        """
        Create size limits with minimum and optional maximum.
        
        Parameters:
        - min: Minimum size (in pages for memory, elements for tables)
        - max: Optional maximum size, None for unbounded
        """
    
    @property
    def min(self) -> int:
        """
        Get the minimum size.
        
        Returns:
        Minimum size constraint
        """
    
    @property
    def max(self) -> Optional[int]:
        """
        Get the maximum size.
        
        Returns:
        Maximum size constraint, or None if unbounded
        """
    
    def __eq__(self, other) -> bool: ...
    def __str__(self) -> str: ...
```

### Import/Export Types

Type declarations for WebAssembly module imports and exports, providing metadata about module interface requirements and capabilities for linking and instantiation.

```python { .api }
class ImportType:
    @property
    def module(self) -> str:
        """
        Get the import module name.
        
        Returns:
        Module name string
        """
    
    @property
    def name(self) -> str:
        """
        Get the import item name.
        
        Returns:
        Import name string
        """
    
    @property
    def type(self) -> Union[FuncType, MemoryType, TableType, GlobalType]:
        """
        Get the import type.
        
        Returns:
        The type of the imported item
        """
    
    def __str__(self) -> str: ...

class ExportType:
    @property
    def name(self) -> str:
        """
        Get the export name.
        
        Returns:
        Export name string
        """
    
    @property
    def type(self) -> Union[FuncType, MemoryType, TableType, GlobalType]:
        """
        Get the export type.
        
        Returns:
        The type of the exported item
        """
    
    def __str__(self) -> str: ...
```

## Usage Examples

### Working with Value Types

```python
import wasmtime

# Create different value types
i32_val = wasmtime.Val.i32(42)
i64_val = wasmtime.Val.i64(1234567890123)
f32_val = wasmtime.Val.f32(3.14159)
f64_val = wasmtime.Val.f64(2.71828182845904)

# Check types and values
print(f"i32 value: {i32_val.value}, type: {i32_val.type}")
print(f"f64 value: {f64_val.value}, type: {f64_val.type}")

# Create reference values
func_ref = wasmtime.Val.funcref(None)  # null function reference
extern_ref = wasmtime.Val.externref({"key": "value"})  # Python object reference
```

### Function Type Definition

```python
import wasmtime

# Define function type: (i32, i32) -> i32
add_type = wasmtime.FuncType(
    [wasmtime.ValType.I32, wasmtime.ValType.I32],
    [wasmtime.ValType.I32]
)

# Define function type: (f64) -> (f64, i32)
multi_result_type = wasmtime.FuncType(
    [wasmtime.ValType.F64],
    [wasmtime.ValType.F64, wasmtime.ValType.I32]
)

# Inspect function types
print(f"Add function params: {add_type.params}")
print(f"Add function results: {add_type.results}")
print(f"Multi-result params: {len(multi_result_type.params)}")
print(f"Multi-result results: {len(multi_result_type.results)}")
```

### Memory and Table Type Specification

```python
import wasmtime

# Create memory type: 1-10 pages (64KB - 640KB)
memory_limits = wasmtime.Limits(1, 10)
memory_type = wasmtime.MemoryType(memory_limits)

# Create unbounded memory type: minimum 2 pages
unbounded_limits = wasmtime.Limits(2)  # No maximum
unbounded_memory_type = wasmtime.MemoryType(unbounded_limits)

# Create table type for function references: 0-100 elements
table_limits = wasmtime.Limits(0, 100)
table_type = wasmtime.TableType(wasmtime.ValType.FUNCREF, table_limits)

# Create table type for external references: minimum 5 elements
extern_table_limits = wasmtime.Limits(5)
extern_table_type = wasmtime.TableType(wasmtime.ValType.EXTERNREF, extern_table_limits)

print(f"Memory limits: {memory_type.limits.min}-{memory_type.limits.max} pages")
print(f"Table element type: {table_type.element}")
```

### Global Variable Types

```python
import wasmtime

# Create mutable i32 global type
mutable_global_type = wasmtime.GlobalType(wasmtime.ValType.I32, True)

# Create immutable f64 global type  
immutable_global_type = wasmtime.GlobalType(wasmtime.ValType.F64, False)

print(f"Mutable global: {mutable_global_type.content}, mutable: {mutable_global_type.mutability}")
print(f"Immutable global: {immutable_global_type.content}, mutable: {immutable_global_type.mutability}")
```

### Module Interface Inspection

```python
import wasmtime

engine = wasmtime.Engine()
module = wasmtime.Module(engine, wasm_bytes)

# Inspect imports
print("Module imports:")
for import_item in module.imports:
    print(f"  {import_item.module}.{import_item.name}: {import_item.type}")

# Inspect exports
print("Module exports:")
for export_item in module.exports:
    print(f"  {export_item.name}: {export_item.type}")
    
    # Check export type
    if isinstance(export_item.type, wasmtime.FuncType):
        func_type = export_item.type
        print(f"    Function: {len(func_type.params)} params, {len(func_type.results)} results")
    elif isinstance(export_item.type, wasmtime.MemoryType):
        mem_type = export_item.type
        print(f"    Memory: {mem_type.limits.min}-{mem_type.limits.max} pages")
```