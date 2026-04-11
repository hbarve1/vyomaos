# Error Handling

Comprehensive error handling system including WebAssembly traps, stack traces, runtime errors, WASI exit codes, and detailed debugging information for robust WebAssembly application development and debugging.

## Capabilities

### Runtime Errors

Base exception hierarchy for WebAssembly runtime errors providing structured error reporting and debugging information for compilation, instantiation, and execution failures.

```python { .api }
class WasmtimeError(Exception):
    """Base exception for all Wasmtime-related errors"""
```

### WebAssembly Traps

Exception representing WebAssembly execution traps with detailed stack trace information, trap codes, and frame-by-frame execution context for debugging runtime failures.

```python { .api }
class Trap(Exception):
    def __init__(self, message: str):
        """
        Create a trap with the given message.
        
        Parameters:
        - message: Human-readable trap description
        """
    
    @property
    def message(self) -> str:
        """
        Get the trap message.
        
        Returns:
        Human-readable trap description
        """
    
    @property
    def frames(self) -> List[Frame]:
        """
        Get the stack trace frames.
        
        Returns:
        List of stack frames from the trap location
        """
    
    @property
    def trace(self) -> List[str]:
        """
        Get the stack trace as formatted strings.
        
        Returns:
        List of formatted stack trace lines
        """
```

### Stack Frames

Stack frame information providing detailed execution context including function names, module names, function indices, and code offsets for precise debugging.

```python { .api }
class Frame:
    @property
    def func_name(self) -> Optional[str]:
        """
        Get the function name if available.
        
        Returns:
        Function name from debug info, or None if not available
        """
    
    @property
    def module_name(self) -> Optional[str]:
        """
        Get the module name if available.
        
        Returns:
        Module name from debug info, or None if not available
        """
    
    @property
    def func_index(self) -> int:
        """
        Get the function index within the module.
        
        Returns:
        Zero-based function index
        """
    
    @property
    def module_offset(self) -> int:
        """
        Get the byte offset within the module.
        
        Returns:
        Byte offset from start of module
        """
```

### Trap Codes

Enumeration of standard WebAssembly trap codes providing categorization of different trap conditions for programmatic error handling and recovery.

```python { .api }
class TrapCode:
    STACK_OVERFLOW: 'TrapCode'           # Stack overflow
    MEMORY_OUT_OF_BOUNDS: 'TrapCode'     # Memory access out of bounds
    HEAP_MISALIGNED: 'TrapCode'          # Misaligned memory access
    TABLE_OUT_OF_BOUNDS: 'TrapCode'      # Table access out of bounds
    INDIRECT_CALL_TO_NULL: 'TrapCode'    # Indirect call to null
    BAD_SIGNATURE: 'TrapCode'            # Indirect call signature mismatch
    INTEGER_OVERFLOW: 'TrapCode'         # Integer overflow
    INTEGER_DIVISION_BY_ZERO: 'TrapCode' # Division by zero
    BAD_CONVERSION_TO_INTEGER: 'TrapCode' # Invalid float to int conversion
    UNREACHABLE: 'TrapCode'              # Unreachable instruction executed
    INTERRUPT: 'TrapCode'                # Execution interrupted
```

### WASI Exit Codes

Exception representing WASI program exit with exit code information, supporting standard Unix exit code conventions and proper process termination handling.

```python { .api }
class ExitTrap(Exception):
    def __init__(self, code: int):
        """
        Create an exit trap with the given exit code.
        
        Parameters:
        - code: Exit code (0 for success, non-zero for error)
        """
    
    @property
    def code(self) -> int:
        """
        Get the exit code.
        
        Returns:
        Exit code from the WebAssembly program
        """
```

## Usage Examples

### Basic Error Handling

```python
import wasmtime

def safe_wasm_execution():
    """Demonstrate basic WebAssembly error handling"""
    
    try:
        engine = wasmtime.Engine()
        store = wasmtime.Store(engine)
        
        # Try to load a WebAssembly module
        wasm_bytes = wasmtime.wat2wasm('''
          (module
            (func (export "divide") (param i32 i32) (result i32)
              local.get 0
              local.get 1
              i32.div_s)  ;; This can trap on division by zero
          )
        ''')
        
        module = wasmtime.Module(engine, wasm_bytes)
        instance = wasmtime.Instance(store, module, [])
        
        # Get the divide function
        divide_func = instance.exports(store)["divide"]
        
        # Safe division
        result = divide_func(store, 10, 2)
        print(f"10 / 2 = {result}")
        
        # This will cause a trap
        result = divide_func(store, 10, 0)
        
    except wasmtime.Trap as trap:
        print(f"WebAssembly trap: {trap.message}")
        print("Stack trace:")
        for frame in trap.frames:
            func_name = frame.func_name or f"func[{frame.func_index}]"
            module_name = frame.module_name or "unknown"
            print(f"  at {func_name} in {module_name} (offset: {frame.module_offset})")
            
    except wasmtime.WasmtimeError as error:
        print(f"Wasmtime error: {error}")
        
    except Exception as error:
        print(f"Unexpected error: {error}")

safe_wasm_execution()
```

### Comprehensive Error Analysis

```python
import wasmtime

def analyze_wasm_error(error):
    """Analyze and categorize WebAssembly errors"""
    
    if isinstance(error, wasmtime.Trap):
        print(f"TRAP: {error.message}")
        
        # Analyze stack trace
        if error.frames:
            print(f"Stack depth: {len(error.frames)} frames")
            for i, frame in enumerate(error.frames):
                print(f"  Frame {i}:")
                print(f"    Function: {frame.func_name or f'func[{frame.func_index}]'}")
                print(f"    Module: {frame.module_name or 'unknown'}")
                print(f"    Offset: 0x{frame.module_offset:x}")
        else:
            print("  No stack trace available")
            
        # Check for common trap patterns
        message_lower = error.message.lower()
        if "division by zero" in message_lower:
            print("  → This is a division by zero error")
            print("  → Check divisor values before division")
        elif "out of bounds" in message_lower:
            print("  → This is a bounds check failure")
            print("  → Verify array/memory access indices")
        elif "unreachable" in message_lower:
            print("  → Unreachable code was executed")
            print("  → Check control flow logic")
            
    elif isinstance(error, wasmtime.ExitTrap):
        print(f"EXIT: Program exited with code {error.code}")
        if error.code == 0:
            print("  → Normal termination")
        else:
            print("  → Error termination")
            
    elif isinstance(error, wasmtime.WasmtimeError):
        print(f"RUNTIME ERROR: {error}")
        
        # Check for common runtime errors
        error_str = str(error).lower()
        if "validation" in error_str:
            print("  → WebAssembly validation failed")
            print("  → Check module binary format and contents")
        elif "import" in error_str:
            print("  → Import resolution failed")
            print("  → Verify all required imports are provided")
        elif "type" in error_str:
            print("  → Type mismatch error")
            print("  → Check function signatures and value types")
            
    else:
        print(f"OTHER ERROR: {type(error).__name__}: {error}")

# Example usage with different error types
def test_various_errors():
    """Test different types of WebAssembly errors"""
    
    engine = wasmtime.Engine()
    store = wasmtime.Store(engine)
    
    # Test 1: Division by zero trap
    try:
        wasm_bytes = wasmtime.wat2wasm('(module (func (export "div") (param i32 i32) (result i32) local.get 0 local.get 1 i32.div_s))')
        module = wasmtime.Module(engine, wasm_bytes)
        instance = wasmtime.Instance(store, module, [])
        div_func = instance.exports(store)["div"]
        div_func(store, 10, 0)  # Division by zero
    except Exception as e:
        print("=== Division by Zero Test ===")
        analyze_wasm_error(e)
        print()
    
    # Test 2: Memory out of bounds
    try:
        wasm_bytes = wasmtime.wat2wasm('''
          (module
            (memory 1)
            (func (export "read_oob") (param i32) (result i32)
              local.get 0
              i32.load))
        ''')
        module = wasmtime.Module(engine, wasm_bytes)
        instance = wasmtime.Instance(store, module, [])
        read_func = instance.exports(store)["read_oob"]
        read_func(store, 100000)  # Way out of bounds
    except Exception as e:
        print("=== Out of Bounds Test ===")
        analyze_wasm_error(e)
        print()
    
    # Test 3: Import resolution error
    try:
        wasm_bytes = wasmtime.wat2wasm('''
          (module
            (import "env" "missing_func" (func (param i32) (result i32)))
            (func (export "test") (result i32)
              i32.const 42
              call 0))
        ''')
        module = wasmtime.Module(engine, wasm_bytes)
        instance = wasmtime.Instance(store, module, [])  # Missing import
    except Exception as e:
        print("=== Import Resolution Test ===")
        analyze_wasm_error(e)
        print()

test_various_errors()
```

### WASI Exit Code Handling

```python
import wasmtime

def run_wasi_program_with_exit_handling(wasm_path: str):
    """Run a WASI program with proper exit code handling"""
    
    try:
        # Set up WASI environment
        wasi_config = wasmtime.WasiConfig()
        wasi_config.inherit_argv()
        wasi_config.inherit_env()
        wasi_config.inherit_stdin()
        wasi_config.inherit_stdout()
        wasi_config.inherit_stderr()
        
        engine = wasmtime.Engine()
        store = wasmtime.Store(engine)
        linker = wasmtime.Linker(engine)
        linker.define_wasi(store, wasi_config)
        
        # Load and instantiate WASI module
        module = wasmtime.Module.from_file(engine, wasm_path)
        instance = linker.instantiate(store, module)
        
        # Run the program
        start_func = instance.get_export(store, "_start")
        if start_func:
            print(f"Running WASI program: {wasm_path}")
            start_func(store)
            print("Program completed successfully (exit code 0)")
            return 0
        else:
            print("Error: No _start function found")
            return 1
            
    except wasmtime.ExitTrap as exit_trap:
        exit_code = exit_trap.code
        if exit_code == 0:
            print("Program completed successfully (explicit exit 0)")
        else:
            print(f"Program exited with error code: {exit_code}")
        return exit_code
        
    except wasmtime.Trap as trap:
        print(f"Program trapped: {trap.message}")
        
        # Print detailed stack trace
        if trap.frames:
            print("Stack trace:")
            for i, frame in enumerate(trap.frames):
                func_name = frame.func_name or f"func[{frame.func_index}]"
                module_name = frame.module_name or "unknown"
                print(f"  {i}: {func_name} in {module_name} at 0x{frame.module_offset:x}")
        
        return 2  # Trap exit code
        
    except wasmtime.WasmtimeError as error:
        print(f"Runtime error: {error}")
        return 3  # Runtime error exit code
        
    except FileNotFoundError:
        print(f"Error: File not found: {wasm_path}")
        return 4  # File not found exit code
        
    except Exception as error:
        print(f"Unexpected error: {error}")
        return 5  # Unexpected error exit code

# Example usage
# exit_code = run_wasi_program_with_exit_handling("my_program.wasm")
# sys.exit(exit_code)
```

### Custom Error Recovery

```python
import wasmtime
import time

class WasmExecutor:
    def __init__(self):
        self.engine = wasmtime.Engine()
        self.retry_count = 0
        self.max_retries = 3
        
    def safe_execute(self, wasm_bytes: bytes, func_name: str, *args):
        """Execute WebAssembly function with error recovery"""
        
        for attempt in range(self.max_retries + 1):
            try:
                store = wasmtime.Store(self.engine)
                module = wasmtime.Module(self.engine, wasm_bytes)
                instance = wasmtime.Instance(store, module, [])
                
                func = instance.exports(store)[func_name]
                result = func(store, *args)
                
                # Reset retry count on success
                self.retry_count = 0
                return result
                
            except wasmtime.Trap as trap:
                print(f"Attempt {attempt + 1}: Trapped - {trap.message}")
                
                # Check if this is a recoverable error
                if self._is_recoverable_trap(trap):
                    if attempt < self.max_retries:
                        wait_time = (2 ** attempt) * 0.1  # Exponential backoff
                        print(f"Retrying in {wait_time:.1f} seconds...")
                        time.sleep(wait_time)
                        continue
                
                # Non-recoverable or max retries exceeded
                print(f"Giving up after {attempt + 1} attempts")
                raise
                
            except wasmtime.WasmtimeError as error:
                print(f"Attempt {attempt + 1}: Runtime error - {error}")
                
                # Most runtime errors are not recoverable
                if "fuel" in str(error).lower() and attempt < self.max_retries:
                    print("Fuel exhausted, retrying with more fuel...")
                    continue
                else:
                    raise
    
    def _is_recoverable_trap(self, trap: wasmtime.Trap) -> bool:
        """Determine if a trap might be recoverable with retry"""
        message = trap.message.lower()
        
        # Some traps might be recoverable (this is application-specific)
        recoverable_patterns = [
            "interrupt",      # Execution interrupted
            "epoch",         # Epoch deadline reached
            "fuel"           # Fuel exhausted
        ]
        
        return any(pattern in message for pattern in recoverable_patterns)

# Example usage
executor = WasmExecutor()

try:
    wasm_bytes = wasmtime.wat2wasm('''
      (module
        (func (export "compute") (param i32) (result i32)
          local.get 0
          i32.const 2
          i32.mul))
    ''')
    
    result = executor.safe_execute(wasm_bytes, "compute", 21)
    print(f"Result: {result}")
    
except Exception as final_error:
    print(f"Final error after all retry attempts: {final_error}")
```

### Error Logging and Debugging

```python
import wasmtime
import logging
import traceback
from datetime import datetime

# Configure logging
logging.basicConfig(
    level=logging.DEBUG,
    format='%(asctime)s - %(levelname)s - %(message)s'
)
logger = logging.getLogger(__name__)

class DebugWasmRunner:
    def __init__(self, enable_debug: bool = True):
        # Create engine with debug info if available
        config = wasmtime.Config()
        if enable_debug:
            config.debug_info(True)
        
        self.engine = wasmtime.Engine(config)
        self.execution_log = []
        
    def run_with_logging(self, wasm_path: str, func_name: str, *args):
        """Run WebAssembly function with comprehensive logging"""
        
        execution_id = datetime.now().isoformat()
        logger.info(f"[{execution_id}] Starting WebAssembly execution")
        logger.info(f"  Module: {wasm_path}")
        logger.info(f"  Function: {func_name}")
        logger.info(f"  Arguments: {args}")
        
        try:
            store = wasmtime.Store(self.engine)
            
            # Load module with validation logging
            logger.debug("Loading WebAssembly module...")
            module = wasmtime.Module.from_file(self.engine, wasm_path)
            logger.debug(f"Module loaded: {len(module.imports)} imports, {len(module.exports)} exports")
            
            # Log imports and exports
            for imp in module.imports:
                logger.debug(f"  Import: {imp.module}.{imp.name} ({imp.type})")
            for exp in module.exports:
                logger.debug(f"  Export: {exp.name} ({exp.type})")
            
            # Instantiate with logging
            logger.debug("Instantiating module...")
            instance = wasmtime.Instance(store, module, [])
            logger.debug("Module instantiated successfully")
            
            # Execute function
            logger.debug(f"Calling function {func_name}...")
            func = instance.exports(store)[func_name]
            result = func(store, *args)
            
            logger.info(f"[{execution_id}] Execution completed successfully")
            logger.info(f"  Result: {result}")
            
            # Record successful execution
            self.execution_log.append({
                "id": execution_id,
                "status": "success",
                "module": wasm_path,
                "function": func_name,
                "args": args,
                "result": result,
                "error": None
            })
            
            return result
            
        except Exception as error:
            logger.error(f"[{execution_id}] Execution failed: {type(error).__name__}: {error}")
            
            # Detailed error logging
            if isinstance(error, wasmtime.Trap):
                logger.error(f"  Trap message: {error.message}")
                logger.error(f"  Stack frames: {len(error.frames)}")
                for i, frame in enumerate(error.frames):
                    logger.error(f"    Frame {i}: {frame.func_name or f'func[{frame.func_index}]'} "
                               f"in {frame.module_name or 'unknown'} at 0x{frame.module_offset:x}")
                    
            elif isinstance(error, wasmtime.ExitTrap):
                logger.error(f"  Exit code: {error.code}")
                
            elif isinstance(error, wasmtime.WasmtimeError):
                logger.error(f"  Runtime error details: {error}")
            
            # Log full Python traceback for debugging
            logger.debug("Python traceback:")
            logger.debug(traceback.format_exc())
            
            # Record failed execution
            self.execution_log.append({
                "id": execution_id,
                "status": "error",
                "module": wasm_path,
                "function": func_name,
                "args": args,
                "result": None,
                "error": {
                    "type": type(error).__name__,
                    "message": str(error),
                    "details": self._extract_error_details(error)
                }
            })
            
            raise
    
    def _extract_error_details(self, error):
        """Extract detailed error information for logging"""
        details = {}
        
        if isinstance(error, wasmtime.Trap):
            details["trap_message"] = error.message
            details["stack_frames"] = []
            for frame in error.frames:
                details["stack_frames"].append({
                    "func_name": frame.func_name,
                    "module_name": frame.module_name,
                    "func_index": frame.func_index,
                    "module_offset": frame.module_offset
                })
        elif isinstance(error, wasmtime.ExitTrap):
            details["exit_code"] = error.code
            
        return details
    
    def get_execution_summary(self):
        """Get summary of all executions"""
        total = len(self.execution_log)
        successful = sum(1 for log in self.execution_log if log["status"] == "success")
        failed = total - successful
        
        return {
            "total_executions": total,
            "successful": successful,
            "failed": failed,
            "success_rate": successful / total if total > 0 else 0,
            "recent_errors": [log["error"] for log in self.execution_log[-5:] if log["error"]]
        }

# Example usage
runner = DebugWasmRunner(enable_debug=True)

try:
    result = runner.run_with_logging("test.wasm", "add", 5, 3)
    print(f"Execution result: {result}")
except Exception as e:
    print(f"Execution failed: {e}")

# Print execution summary
summary = runner.get_execution_summary()
print(f"Execution summary: {summary}")
```