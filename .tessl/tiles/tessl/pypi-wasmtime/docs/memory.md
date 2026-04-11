# Memory Management

Linear memory allocation, access, and manipulation supporting both regular and shared memory models. Provides comprehensive memory operations including growth, data transfer, multi-threading coordination, and direct memory access for high-performance WebAssembly applications.

## Capabilities

### Linear Memory

WebAssembly linear memory providing byte-addressable storage with dynamic growth capabilities, bounds checking, and direct data access for efficient communication between Python and WebAssembly code.

```python { .api }
class Memory:
    def __init__(self, store: Store, ty: MemoryType):
        """
        Create a new linear memory with specified type.
        
        Parameters:
        - store: Store for memory allocation context
        - ty: Memory type specifying size limits
        
        Raises:
        WasmtimeError: If memory allocation fails
        """
    
    def type(self, store: Store) -> MemoryType:
        """
        Get the memory type specification.
        
        Parameters:
        - store: Store context
        
        Returns:
        Memory type with current size limits
        """
    
    def grow(self, store: Store, delta: int) -> int:
        """
        Grow memory by specified number of pages.
        
        Parameters:
        - store: Store context
        - delta: Number of 64KB pages to add (must be non-negative)
        
        Returns:
        Previous memory size in pages
        
        Raises:
        WasmtimeError: If growth fails or exceeds maximum size
        """
    
    def size(self, store: Store) -> int:
        """
        Get current memory size in pages.
        
        Parameters:
        - store: Store context
        
        Returns:
        Memory size in 64KB pages
        """
    
    def data_len(self, store: Store) -> int:
        """
        Get current memory size in bytes.
        
        Parameters:
        - store: Store context
        
        Returns:
        Memory size in bytes
        """
    
    def data_ptr(self, store: Store) -> int:
        """
        Get pointer to raw memory data.
        
        Parameters:
        - store: Store context
        
        Returns:
        Pointer to memory data (for advanced use with ctypes)
        
        Warning:
        Direct pointer access bypasses bounds checking
        """
    
    def read(self, store: Store, start: int, stop: int) -> bytes:
        """
        Read bytes from memory range.
        
        Parameters:
        - store: Store context
        - start: Starting byte offset
        - stop: Ending byte offset (exclusive)
        
        Returns:
        Bytes read from memory
        
        Raises:
        WasmtimeError: If range is out of bounds
        """
    
    def write(self, store: Store, data: bytes, start: int = 0) -> None:
        """
        Write bytes to memory starting at offset.
        
        Parameters:
        - store: Store context
        - data: Bytes to write
        - start: Starting byte offset (default: 0)
        
        Raises:
        WasmtimeError: If write would exceed memory bounds
        """
```

### Shared Memory

Multi-threaded shared memory supporting concurrent access from multiple WebAssembly instances, with proper synchronization primitives and cross-thread memory coordination.

```python { .api }
class SharedMemory:
    def __init__(self, engine: Engine, ty: MemoryType):
        """
        Create a new shared memory with specified type.
        
        Parameters:
        - engine: Engine for shared memory allocation
        - ty: Memory type specifying size limits (must be shared-compatible)
        
        Raises:
        WasmtimeError: If shared memory creation fails or type incompatible
        """
    
    def type(self) -> MemoryType:
        """
        Get the shared memory type specification.
        
        Returns:
        Memory type with size limits and shared flag
        """
    
    def as_memory(self, store: Store) -> Memory:
        """
        Get a Memory view for use within a specific store.
        
        Parameters:
        - store: Store context for memory access
        
        Returns:
        Memory object providing access to shared memory
        
        Note:
        Multiple stores can have Memory views of the same SharedMemory
        """
```

## Usage Examples

### Basic Memory Operations

```python
import wasmtime

# Create memory type: 1-10 pages (64KB - 640KB)
limits = wasmtime.Limits(1, 10)
memory_type = wasmtime.MemoryType(limits)

# Create engine, store, and memory
engine = wasmtime.Engine()
store = wasmtime.Store(engine)
memory = wasmtime.Memory(store, memory_type)

# Check initial memory size
pages = memory.size(store)
bytes_size = memory.data_len(store)
print(f"Initial memory: {pages} pages ({bytes_size} bytes)")

# Write data to memory
message = b"Hello from Python!"
memory.write(store, message, 0)

# Read data back
read_data = memory.read(store, 0, len(message))
print(f"Read from memory: {read_data.decode('utf-8')}")

# Grow memory
old_size = memory.grow(store, 2)  # Add 2 pages (128KB)
new_size = memory.size(store)
print(f"Memory grown from {old_size} to {new_size} pages")
```

### Memory with WebAssembly Module

```python
import wasmtime

# WebAssembly module that uses memory
wasm_bytes = wasmtime.wat2wasm('''
  (module
    (memory (export "memory") 1 10)
    (func (export "write_hello") (param i32)
      ;; Write "Hello" starting at given offset
      local.get 0
      i32.const 72  ;; 'H'
      i32.store8
      local.get 0
      i32.const 1
      i32.add
      i32.const 101 ;; 'e'
      i32.store8
      local.get 0
      i32.const 2
      i32.add
      i32.const 108 ;; 'l'
      i32.store8
      local.get 0
      i32.const 3
      i32.add
      i32.const 108 ;; 'l'
      i32.store8
      local.get 0
      i32.const 4
      i32.add
      i32.const 111 ;; 'o'
      i32.store8)
    (func (export "read_byte") (param i32) (result i32)
      local.get 0
      i32.load8_u)
  )
''')

engine = wasmtime.Engine()
store = wasmtime.Store(engine)
module = wasmtime.Module(engine, wasm_bytes)
instance = wasmtime.Instance(store, module, [])

# Get exported memory and functions
exports = instance.exports(store)
memory = exports["memory"]
write_hello = exports["write_hello"]
read_byte = exports["read_byte"]

# Call WebAssembly function to write to memory
write_hello(store, 10)  # Write "Hello" starting at offset 10

# Read from memory using Python
hello_bytes = memory.read(store, 10, 15)
print(f"WebAssembly wrote: {hello_bytes.decode('utf-8')}")

# Read individual bytes using WebAssembly function
for i in range(5):
    byte_val = read_byte(store, 10 + i)
    print(f"Byte at {10 + i}: {byte_val} ('{chr(byte_val)}')")
```

### Shared Memory for Multi-threading

```python
import wasmtime
import threading
import time

# Create shared memory configuration
config = wasmtime.Config()
config.wasm_threads(True)  # Enable threads support
engine = wasmtime.Engine(config)

# Create shared memory: 1-5 pages, shared
limits = wasmtime.Limits(1, 5)
memory_type = wasmtime.MemoryType(limits)
shared_memory = wasmtime.SharedMemory(engine, memory_type)

def worker_thread(thread_id: int):
    """Worker thread that accesses shared memory"""
    # Each thread needs its own store
    store = wasmtime.Store(engine)
    
    # Get memory view for this store
    memory = shared_memory.as_memory(store)
    
    # Write thread-specific data
    message = f"Thread {thread_id} was here!".encode('utf-8')
    offset = thread_id * 32  # Each thread writes to different offset
    memory.write(store, message, offset)
    
    print(f"Thread {thread_id} wrote to offset {offset}")

# Start multiple threads
threads = []
for i in range(3):
    thread = threading.Thread(target=worker_thread, args=(i,))
    threads.append(thread)
    thread.start()

# Wait for all threads to complete
for thread in threads:
    thread.join()

# Read all data from main thread
main_store = wasmtime.Store(engine)
main_memory = shared_memory.as_memory(main_store)

print("Reading results from shared memory:")
for i in range(3):
    offset = i * 32
    # Read up to 32 bytes or until null terminator
    data = main_memory.read(main_store, offset, offset + 32)
    # Find null terminator if present
    null_pos = data.find(b'\x00')
    if null_pos != -1:
        data = data[:null_pos]
    print(f"Thread {i} data: {data.decode('utf-8')}")
```

### Memory Growth and Bounds Checking

```python
import wasmtime

# Create memory with tight limits for demonstration
limits = wasmtime.Limits(1, 3)  # 1-3 pages (64KB - 192KB)
memory_type = wasmtime.MemoryType(limits)

engine = wasmtime.Engine()
store = wasmtime.Store(engine)
memory = wasmtime.Memory(store, memory_type)

# Fill initial page with data
page_size = 64 * 1024  # 64KB
test_data = b'A' * (page_size - 100)  # Fill most of first page
memory.write(store, test_data, 0)

print(f"Initial memory: {memory.size(store)} pages")

# Try to write beyond current memory - should fail
try:
    memory.write(store, b"This will fail", page_size + 1000)
except wasmtime.WasmtimeError as e:
    print(f"Expected error: {e}")

# Grow memory and try again
old_size = memory.grow(store, 1)  # Add 1 page
print(f"Memory grown from {old_size} to {memory.size(store)} pages")

# Now write to second page - should succeed
memory.write(store, b"Second page data", page_size + 100)
second_page_data = memory.read(store, page_size + 100, page_size + 116)
print(f"Second page contains: {second_page_data.decode('utf-8')}")

# Try to grow beyond maximum - should fail
try:
    memory.grow(store, 5)  # Would exceed 3-page maximum
except wasmtime.WasmtimeError as e:
    print(f"Growth limit reached: {e}")
```

### Direct Memory Access with ctypes

```python
import wasmtime
import ctypes

# Create memory
engine = wasmtime.Engine()
store = wasmtime.Store(engine)
limits = wasmtime.Limits(1)
memory_type = wasmtime.MemoryType(limits)
memory = wasmtime.Memory(store, memory_type)

# Get direct pointer to memory (advanced usage)
ptr = memory.data_ptr(store)
size = memory.data_len(store)

# Create ctypes array from memory pointer
# WARNING: This bypasses WebAssembly bounds checking!
memory_array = (ctypes.c_ubyte * size).from_address(ptr)

# Write using ctypes (very fast, but unsafe)
test_string = b"Direct memory access"
for i, byte in enumerate(test_string):
    memory_array[i] = byte

# Read back using normal memory API
read_data = memory.read(store, 0, len(test_string))
print(f"Direct write result: {read_data.decode('utf-8')}")

# Structure overlay example
class DataStruct(ctypes.Structure):
    _fields_ = [
        ("magic", ctypes.c_uint32),
        ("version", ctypes.c_uint16),
        ("flags", ctypes.c_uint16),
        ("data_size", ctypes.c_uint32)
    ]

# Overlay structure on memory (at offset 100)
struct_ptr = ptr + 100
data_struct = DataStruct.from_address(struct_ptr)

# Write structured data
data_struct.magic = 0x12345678
data_struct.version = 1
data_struct.flags = 0b1010
data_struct.data_size = 1024

# Read back using memory API
struct_bytes = memory.read(store, 100, 100 + ctypes.sizeof(DataStruct))
magic_bytes = struct_bytes[:4]
magic_value = int.from_bytes(magic_bytes, 'little')
print(f"Magic value: 0x{magic_value:08x}")
```

### Memory Debugging and Inspection

```python
import wasmtime

def dump_memory(memory: wasmtime.Memory, store: wasmtime.Store, 
                start: int = 0, length: int = 256, width: int = 16):
    """Debug utility to dump memory contents in hex format"""
    data = memory.read(store, start, start + length)
    
    print(f"Memory dump from 0x{start:04x} to 0x{start + length:04x}:")
    for i in range(0, len(data), width):
        # Address
        addr = start + i
        print(f"{addr:04x}: ", end="")
        
        # Hex bytes
        line_data = data[i:i + width]
        for j, byte in enumerate(line_data):
            print(f"{byte:02x} ", end="")
        
        # Padding for incomplete lines
        for j in range(len(line_data), width):
            print("   ", end="")
        
        # ASCII representation
        print(" |", end="")
        for byte in line_data:
            if 32 <= byte <= 126:  # Printable ASCII
                print(chr(byte), end="")
            else:
                print(".", end="")
        print("|")

# Example usage
engine = wasmtime.Engine()
store = wasmtime.Store(engine)
limits = wasmtime.Limits(1)
memory = wasmtime.Memory(store, wasmtime.MemoryType(limits))

# Write some test data
test_data = b"Hello, WebAssembly World!\x00\x01\x02\x03\xff\xfe\xfd"
memory.write(store, test_data, 0)

# Dump memory contents
dump_memory(memory, store, 0, 64)
```