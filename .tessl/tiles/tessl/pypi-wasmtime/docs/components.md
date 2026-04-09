# Component Model

WebAssembly component model integration through bindgen tooling, enabling rich type communication between Python and WebAssembly components with automatic binding generation, interface definitions, and high-level abstractions for complex data types.

## Capabilities

### Bindgen Code Generation

Automatic Python binding generation from WebAssembly components, providing seamless integration between Python and component model interfaces with type-safe communication and automatic marshalling.

```python { .api }
import wasmtime.bindgen

def generate(name: str, component: bytes) -> Dict[str, bytes]:
    """
    Generate Python bindings from a WebAssembly component.
    
    Parameters:
    - name: Name for the generated bindings (used as module name)
    - component: Binary WebAssembly component data
    
    Returns:
    Dictionary mapping file names to generated Python code bytes
    
    Raises:
    RuntimeError: If component parsing or binding generation fails
    """
```

### Component Runtime

Runtime system for WebAssembly components providing execution environment, import/export management, and interface implementation for component model applications.

```python { .api }
class Root:
    """Generated root component binding class"""
    def __init__(self, store: Store, imports: RootImports): ...

class RootImports:
    """Generated import interface definitions"""
    def __init__(self, *host_implementations): ...

class Err:
    """Error result type for component operations"""
    @property
    def value(self): ...
```

### WASI Component Implementations

Host implementations of WASI component interfaces providing system integration, I/O operations, filesystem access, and environment interaction for component model applications.

```python { .api }
# Stream implementations
class WasiStreams:
    def drop_input_stream(self, this: int) -> None: ...
    def write(self, this: int, buf: bytes) -> Tuple[int, int]: ...
    def blocking_write(self, this: int, buf: bytes) -> Tuple[int, int]: ...
    def drop_output_stream(self, this: int) -> None: ...

# Filesystem implementations  
class WasiTypes:
    def write_via_stream(self, this: int, offset: int): ...
    def append_via_stream(self, this: int): ...
    def get_type(self, this: int): ...
    def drop_descriptor(self, this: int) -> None: ...

# Environment implementations
class WasiEnvironment:
    def get_environment(self) -> List[Tuple[str, str]]: ...

class WasiPreopens:
    def get_directories(self) -> List[Tuple[int, str]]: ...

# Standard I/O implementations
class WasiStdin:
    def get_stdin(self) -> int: ...

class WasiStdout:
    def get_stdout(self) -> int: ...

class WasiStderr:  
    def get_stderr(self) -> int: ...

# Random number generation
class WasiRandom:
    def get_random_bytes(self, len: int) -> bytes: ...

# Process control
class WasiExit:
    def exit(self, status) -> None: ...

# Terminal interfaces
class WasiTerminalInput:
    def drop_terminal_input(self, this: int) -> None: ...

class WasiTerminalOutput:
    def drop_terminal_output(self, this: int) -> None: ...

class WasiTerminalStdin:
    def get_terminal_stdin(self) -> Optional[int]: ...

class WasiTerminalStdout:
    def get_terminal_stdout(self) -> Optional[int]: ...

class WasiTerminalStderr:
    def get_terminal_stderr(self) -> Optional[int]: ...
```

## Usage Examples

### Basic Component Binding Generation

```python
import wasmtime.bindgen

# Load a WebAssembly component
with open("my_component.wasm", "rb") as f:
    component_bytes = f.read()

# Generate Python bindings
bindings = wasmtime.bindgen.generate("my_component", component_bytes)

# The result is a dictionary of file names to Python code
for filename, code_bytes in bindings.items():
    print(f"Generated file: {filename}")
    if filename.endswith('.py'):
        print("Python code preview:")
        print(code_bytes.decode('utf-8')[:200] + "...")
    print()

# Save generated bindings to disk
import os
output_dir = "generated_bindings"
os.makedirs(output_dir, exist_ok=True)

for filename, code_bytes in bindings.items():
    filepath = os.path.join(output_dir, filename)
    with open(filepath, 'wb') as f:
        f.write(code_bytes)
    print(f"Saved: {filepath}")
```

### Component Execution with Custom Host Implementations

```python
import wasmtime
import wasmtime.bindgen
import sys
import os
from typing import List, Tuple, Optional

# Custom host implementations for WASI interfaces
class MyWasiStreams(wasmtime.bindgen.WasiStreams):
    def __init__(self):
        self.output_buffer = []
        
    def write(self, stream_id: int, buf: bytes) -> Tuple[int, int]:
        """Write to stream and return (bytes_written, status)"""
        if stream_id == 1:  # stdout
            sys.stdout.buffer.write(buf)
            sys.stdout.buffer.flush()
        elif stream_id == 2:  # stderr
            sys.stderr.buffer.write(buf)
            sys.stderr.buffer.flush()
        else:
            # Custom stream - store in buffer
            self.output_buffer.append((stream_id, buf))
        
        return (len(buf), 0)  # 0 = stream open status
    
    def blocking_write(self, stream_id: int, buf: bytes) -> Tuple[int, int]:
        return self.write(stream_id, buf)

class MyWasiEnvironment(wasmtime.bindgen.WasiEnvironment):
    def __init__(self, custom_vars: dict = None):
        self.custom_vars = custom_vars or {}
        
    def get_environment(self) -> List[Tuple[str, str]]:
        """Return environment variables as (key, value) pairs"""
        env_vars = []
        
        # Add system environment variables (filtered)
        for key, value in os.environ.items():
            if not key.startswith(('SECRET_', 'TOKEN_')):
                env_vars.append((key, value))
        
        # Add custom variables
        for key, value in self.custom_vars.items():
            env_vars.append((key, value))
            
        return env_vars

class MyWasiRandom(wasmtime.bindgen.WasiRandom):
    def get_random_bytes(self, length: int) -> bytes:
        """Generate cryptographically secure random bytes"""
        return os.urandom(length)

# Set up component execution
def run_component_with_custom_hosts(component_path: str):
    """Run a WebAssembly component with custom host implementations"""
    
    # Load component
    with open(component_path, 'rb') as f:
        component_bytes = f.read()
    
    # Generate bindings
    bindings = wasmtime.bindgen.generate("component", component_bytes)
    
    # Initialize component runtime
    root, store = wasmtime.bindgen.init()
    
    # Create custom host implementations
    custom_streams = MyWasiStreams()
    custom_env = MyWasiEnvironment({
        "COMPONENT_MODE": "python_host",
        "MAX_BUFFER_SIZE": "1048576"
    })
    custom_random = MyWasiRandom()
    
    # Use default implementations for other interfaces
    default_types = wasmtime.bindgen.WasiTypes()
    default_preopens = wasmtime.bindgen.WasiPreopens()
    default_exit = wasmtime.bindgen.WasiExit()
    default_stdin = wasmtime.bindgen.WasiStdin()
    default_stdout = wasmtime.bindgen.WasiStdout()
    default_stderr = wasmtime.bindgen.WasiStderr()
    default_terminal_input = wasmtime.bindgen.WasiTerminalInput()
    default_terminal_output = wasmtime.bindgen.WasiTerminalOutput()
    default_terminal_stdin = wasmtime.bindgen.WasiTerminalStdin()
    default_terminal_stdout = wasmtime.bindgen.WasiTerminalStdout()
    default_terminal_stderr = wasmtime.bindgen.WasiTerminalStderr()
    
    # Create root imports with custom implementations
    imports = wasmtime.bindgen.RootImports(
        custom_streams,
        default_types,
        default_preopens,
        custom_random,
        custom_env,
        default_exit,
        default_stdin,
        default_stdout,
        default_stderr,
        default_terminal_input,
        default_terminal_output,
        default_terminal_stdin,
        default_terminal_stdout,
        default_terminal_stderr
    )
    
    # Create component instance
    component = wasmtime.bindgen.Root(store, imports)
    
    print("Component initialized with custom host implementations")
    print(f"Custom environment variables: {len(custom_env.custom_vars)}")
    
    return component, store

# Example usage
# component, store = run_component_with_custom_hosts("my_component.wasm")
```

### Advanced Component Interface Implementation

```python
import wasmtime.bindgen
from typing import Dict, Any, List, Tuple
import json
import logging

class AdvancedWasiStreams(wasmtime.bindgen.WasiStreams):
    """Advanced stream implementation with logging and buffering"""
    
    def __init__(self):
        self.streams: Dict[int, Dict[str, Any]] = {}
        self.next_stream_id = 3  # Start after stdout/stderr
        self.logger = logging.getLogger(__name__)
        
    def create_stream(self, name: str, buffer_size: int = 4096) -> int:
        """Create a new custom stream"""
        stream_id = self.next_stream_id
        self.next_stream_id += 1
        
        self.streams[stream_id] = {
            "name": name,
            "buffer": bytearray(),
            "buffer_size": buffer_size,
            "total_written": 0
        }
        
        self.logger.info(f"Created stream {stream_id}: {name}")
        return stream_id
    
    def write(self, stream_id: int, buf: bytes) -> Tuple[int, int]:
        """Write to stream with detailed logging"""
        self.logger.debug(f"Writing {len(buf)} bytes to stream {stream_id}")
        
        if stream_id == 1:  # stdout
            import sys
            sys.stdout.buffer.write(buf)
            sys.stdout.buffer.flush()
            return (len(buf), 0)
            
        elif stream_id == 2:  # stderr
            import sys
            sys.stderr.buffer.write(buf)
            sys.stderr.buffer.flush()
            return (len(buf), 0)
            
        elif stream_id in self.streams:
            # Custom stream
            stream = self.streams[stream_id]
            stream["buffer"].extend(buf)
            stream["total_written"] += len(buf)
            
            # Log buffer status
            self.logger.debug(f"Stream {stream_id} ({stream['name']}): "
                            f"{len(stream['buffer'])} buffered, "
                            f"{stream['total_written']} total")
            
            # Auto-flush if buffer is full
            if len(stream["buffer"]) >= stream["buffer_size"]:
                self.flush_stream(stream_id)
            
            return (len(buf), 0)
        else:
            self.logger.error(f"Unknown stream ID: {stream_id}")
            return (0, 1)  # Error status
    
    def flush_stream(self, stream_id: int):
        """Flush a custom stream buffer"""
        if stream_id in self.streams:
            stream = self.streams[stream_id]
            if stream["buffer"]:
                self.logger.info(f"Flushing stream {stream_id} ({stream['name']}): "
                               f"{len(stream['buffer'])} bytes")
                
                # Process buffered data (example: save to file)
                filename = f"stream_{stream_id}_{stream['name']}.log"
                with open(filename, 'ab') as f:
                    f.write(stream["buffer"])
                
                stream["buffer"].clear()

class ConfigurableWasiEnvironment(wasmtime.bindgen.WasiEnvironment):
    """Environment implementation with configuration file support"""
    
    def __init__(self, config_file: str = None):
        self.config = self.load_config(config_file) if config_file else {}
        self.logger = logging.getLogger(__name__)
        
    def load_config(self, config_file: str) -> dict:
        """Load environment configuration from JSON file"""
        try:
            with open(config_file, 'r') as f:
                config = json.load(f)
            self.logger.info(f"Loaded environment config from {config_file}")
            return config
        except Exception as e:
            self.logger.error(f"Failed to load config {config_file}: {e}")
            return {}
    
    def get_environment(self) -> List[Tuple[str, str]]:
        """Get environment variables with configuration override"""
        env_vars = []
        
        # Start with configured base environment
        base_env = self.config.get("base_environment", "inherit")
        
        if base_env == "inherit":
            # Inherit from host process
            import os
            for key, value in os.environ.items():
                if self.is_allowed_var(key):
                    env_vars.append((key, value))
        elif base_env == "minimal":
            # Only essential variables
            essential_vars = ["HOME", "PATH", "USER", "SHELL"]
            import os
            for var in essential_vars:
                if var in os.environ:
                    env_vars.append((var, os.environ[var]))
        
        # Add custom variables from config
        custom_vars = self.config.get("custom_variables", {})
        for key, value in custom_vars.items():
            env_vars.append((key, str(value)))
        
        # Add runtime variables
        env_vars.extend([
            ("WASM_COMPONENT_MODE", "python_bindgen"),
            ("COMPONENT_TIMESTAMP", str(int(__import__('time').time()))),
            ("PYTHON_VERSION", __import__('sys').version.split()[0])
        ])
        
        self.logger.debug(f"Providing {len(env_vars)} environment variables")
        return env_vars
    
    def is_allowed_var(self, var_name: str) -> bool:
        """Check if environment variable is allowed"""
        blocked_patterns = self.config.get("blocked_patterns", [
            "SECRET_", "TOKEN_", "PASSWORD_", "KEY_", "PRIVATE_"
        ])
        
        return not any(var_name.startswith(pattern) for pattern in blocked_patterns)

# Complete component setup with advanced implementations
def setup_advanced_component(component_path: str, config_file: str = None):
    """Set up component with advanced host implementations"""
    
    # Configure logging
    logging.basicConfig(level=logging.INFO)
    logger = logging.getLogger(__name__)
    
    try:
        # Load component
        with open(component_path, 'rb') as f:
            component_bytes = f.read()
        
        logger.info(f"Loaded component: {len(component_bytes)} bytes")
        
        # Generate bindings
        logger.info("Generating Python bindings...")
        bindings = wasmtime.bindgen.generate("advanced_component", component_bytes)
        logger.info(f"Generated {len(bindings)} binding files")
        
        # Initialize runtime
        root, store = wasmtime.bindgen.init()
        
        # Create advanced host implementations
        advanced_streams = AdvancedWasiStreams()
        configurable_env = ConfigurableWasiEnvironment(config_file)
        
        # Create additional custom streams
        log_stream = advanced_streams.create_stream("application_log")
        metrics_stream = advanced_streams.create_stream("metrics")
        
        logger.info(f"Created custom streams: log={log_stream}, metrics={metrics_stream}")
        
        # Set up all WASI interfaces
        imports = wasmtime.bindgen.RootImports(
            advanced_streams,
            wasmtime.bindgen.WasiTypes(),
            wasmtime.bindgen.WasiPreopens(),
            wasmtime.bindgen.WasiRandom(),
            configurable_env,
            wasmtime.bindgen.WasiExit(),
            wasmtime.bindgen.WasiStdin(),
            wasmtime.bindgen.WasiStdout(),
            wasmtime.bindgen.WasiStderr(),
            wasmtime.bindgen.WasiTerminalInput(),
            wasmtime.bindgen.WasiTerminalOutput(),
            wasmtime.bindgen.WasiTerminalStdin(),
            wasmtime.bindgen.WasiTerminalStdout(),
            wasmtime.bindgen.WasiTerminalStderr()
        )
        
        # Create component instance
        component = wasmtime.bindgen.Root(store, imports)
        
        logger.info("Component setup complete")
        return component, store, advanced_streams
        
    except Exception as e:
        logger.error(f"Component setup failed: {e}")
        raise

# Example configuration file (config.json):
example_config = '''
{
  "base_environment": "minimal",
  "custom_variables": {
    "COMPONENT_NAME": "advanced_example",
    "LOG_LEVEL": "info",
    "MAX_MEMORY": "134217728",
    "ENABLE_METRICS": "true"
  },
  "blocked_patterns": [
    "SECRET_",
    "TOKEN_",
    "PASSWORD_",
    "API_KEY_",
    "PRIVATE_"
  ]
}
'''

# Save example config
# with open("component_config.json", "w") as f:
#     f.write(example_config)

# Example usage:
# component, store, streams = setup_advanced_component("my_component.wasm", "component_config.json")
```