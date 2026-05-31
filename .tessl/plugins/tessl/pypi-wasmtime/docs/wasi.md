# WASI Integration

Complete WebAssembly System Interface implementation providing filesystem access, environment variables, command-line arguments, I/O redirection, and comprehensive permission controls for secure WebAssembly execution in system environments.

## Capabilities

### WASI Configuration

Configuration system for WebAssembly System Interface providing fine-grained control over system resource access, I/O redirection, filesystem permissions, and process environment setup.

```python { .api }
class WasiConfig:
    def __init__(self):
        """Create a new WASI configuration with default settings"""
    
    # Command line arguments
    def argv(self, argv: List[str]) -> None:
        """
        Set command line arguments for the WebAssembly program.
        
        Parameters:
        - argv: List of command line arguments (first element is program name)
        """
    
    def inherit_argv(self) -> None:
        """Inherit command line arguments from the current Python process"""
    
    # Environment variables
    def env(self, env: List[Tuple[str, str]]) -> None:
        """
        Set environment variables for the WebAssembly program.
        
        Parameters:
        - env: List of (name, value) tuples for environment variables
        """
    
    def inherit_env(self) -> None:
        """Inherit environment variables from the current Python process"""
    
    # Standard I/O redirection
    def stdin_file(self, path: str) -> None:
        """
        Redirect stdin from a file.
        
        Parameters:
        - path: Path to file for stdin input
        """
    
    def stdout_file(self, path: str) -> None:
        """
        Redirect stdout to a file.
        
        Parameters:
        - path: Path to file for stdout output
        """
    
    def stderr_file(self, path: str) -> None:
        """
        Redirect stderr to a file.
        
        Parameters:
        - path: Path to file for stderr output
        """
    
    def inherit_stdin(self) -> None:
        """Inherit stdin from the current Python process"""
    
    def inherit_stdout(self) -> None:
        """Inherit stdout from the current Python process"""
    
    def inherit_stderr(self) -> None:
        """Inherit stderr from the current Python process"""
    
    # Filesystem access
    def preopen_dir(self, path: str, guest_path: str, perms: DirPerms) -> None:
        """
        Preopen a directory for WebAssembly filesystem access.
        
        Parameters:
        - path: Host filesystem path to directory
        - guest_path: Path as seen by WebAssembly program
        - perms: Directory access permissions
        """
```

### Directory Permissions

Permission enumeration controlling directory access rights for preopened directories, providing read-only, write-only, and read-write access control.

```python { .api }
class DirPerms:
    READ_ONLY: 'DirPerms'    # Read-only directory access
    WRITE_ONLY: 'DirPerms'   # Write-only directory access  
    READ_WRITE: 'DirPerms'   # Read and write directory access
```

### File Permissions

Permission enumeration controlling file access rights within preopened directories, providing granular control over file operations and security boundaries.

```python { .api }
class FilePerms:
    READ_ONLY: 'FilePerms'    # Read-only file access
    WRITE_ONLY: 'FilePerms'   # Write-only file access
    READ_WRITE: 'FilePerms'   # Read and write file access
```

## Usage Examples

### Basic WASI Setup

```python
import wasmtime
import sys
import os

# Create WASI configuration
wasi_config = wasmtime.WasiConfig()

# Configure command line arguments
wasi_config.argv(["my_program", "arg1", "arg2", "--flag"])

# Configure environment variables
wasi_config.env([
    ("HOME", "/home/wasm"),
    ("PATH", "/usr/bin:/bin"),
    ("CUSTOM_VAR", "custom_value")
])

# Inherit standard I/O
wasi_config.inherit_stdin()
wasi_config.inherit_stdout()
wasi_config.inherit_stderr()

# Create engine and linker
engine = wasmtime.Engine()
linker = wasmtime.Linker(engine)
store = wasmtime.Store(engine)

# Define WASI imports
linker.define_wasi(store, wasi_config)

# Load and instantiate WASI-enabled WebAssembly module
module = wasmtime.Module.from_file(engine, "wasi_program.wasm")
instance = linker.instantiate(store, module)

# Call main function if it exists
try:
    start_func = instance.get_export(store, "_start")
    if start_func:
        start_func(store)
except wasmtime.ExitTrap as exit_trap:
    print(f"Program exited with code: {exit_trap.code}")
```

### Filesystem Access with Permissions

```python
import wasmtime
import tempfile
import os

# Create temporary directories for demo
with tempfile.TemporaryDirectory() as temp_dir:
    # Create some test directories
    input_dir = os.path.join(temp_dir, "input")
    output_dir = os.path.join(temp_dir, "output")
    config_dir = os.path.join(temp_dir, "config")
    
    os.makedirs(input_dir)
    os.makedirs(output_dir)  
    os.makedirs(config_dir)
    
    # Create test files
    with open(os.path.join(input_dir, "data.txt"), "w") as f:
        f.write("Input data for processing")
    
    with open(os.path.join(config_dir, "settings.json"), "w") as f:
        f.write('{"debug": true, "max_items": 100}')
    
    # Configure WASI with different directory permissions
    wasi_config = wasmtime.WasiConfig()
    
    # Read-only input directory
    wasi_config.preopen_dir(input_dir, "/input", wasmtime.DirPerms.READ_ONLY)
    
    # Read-write output directory
    wasi_config.preopen_dir(output_dir, "/output", wasmtime.DirPerms.READ_WRITE)
    
    # Read-only config directory
    wasi_config.preopen_dir(config_dir, "/config", wasmtime.DirPerms.READ_ONLY)
    
    # Setup WebAssembly execution
    engine = wasmtime.Engine()
    store = wasmtime.Store(engine)
    linker = wasmtime.Linker(engine)
    linker.define_wasi(store, wasi_config)
    
    # The WebAssembly program can now:
    # - Read files from /input and /config
    # - Write files to /output
    # - Cannot write to /input or /config
    
    print("WASI filesystem access configured:")
    print(f"  /input -> {input_dir} (read-only)")
    print(f"  /output -> {output_dir} (read-write)")
    print(f"  /config -> {config_dir} (read-only)")
```

### I/O Redirection

```python
import wasmtime
import tempfile
import os

# Create temporary files for I/O redirection
with tempfile.NamedTemporaryFile(mode='w', delete=False) as input_file:
    input_file.write("Hello from input file!\n")
    input_file.write("Second line of input\n")
    input_file_path = input_file.name

with tempfile.NamedTemporaryFile(mode='w', delete=False) as output_file:
    output_file_path = output_file.name

with tempfile.NamedTemporaryFile(mode='w', delete=False) as error_file:
    error_file_path = error_file.name

try:
    # Configure WASI with I/O redirection
    wasi_config = wasmtime.WasiConfig()
    
    # Redirect stdin from file
    wasi_config.stdin_file(input_file_path)
    
    # Redirect stdout to file
    wasi_config.stdout_file(output_file_path)
    
    # Redirect stderr to file
    wasi_config.stderr_file(error_file_path)
    
    # Set up command line arguments
    wasi_config.argv(["io_test", "--echo"])
    
    # Set up environment
    wasi_config.env([("TEST_MODE", "redirect")])
    
    # Create WebAssembly execution environment
    engine = wasmtime.Engine()
    store = wasmtime.Store(engine)
    linker = wasmtime.Linker(engine)
    linker.define_wasi(store, wasi_config)
    
    # Load and run WebAssembly program
    # (This would be a WASI program that reads stdin and writes to stdout/stderr)
    # module = wasmtime.Module.from_file(engine, "io_program.wasm")
    # instance = linker.instantiate(store, module)
    # start_func = instance.get_export(store, "_start")
    # start_func(store)
    
    print("I/O redirection configured:")
    print(f"  stdin <- {input_file_path}")
    print(f"  stdout -> {output_file_path}")
    print(f"  stderr -> {error_file_path}")
    
    # After execution, read the output files
    # with open(output_file_path, 'r') as f:
    #     print(f"Program output: {f.read()}")

finally:
    # Clean up temporary files
    for path in [input_file_path, output_file_path, error_file_path]:
        try:
            os.unlink(path)
        except OSError:
            pass
```

### Environment Variable Inheritance and Customization

```python
import wasmtime
import os

# Get current environment
current_env = dict(os.environ)

# Create custom environment based on current environment
custom_env = []
for key, value in current_env.items():
    # Filter out sensitive variables
    if not key.startswith(('SECRET_', 'TOKEN_', 'PASSWORD_')):
        custom_env.append((key, value))

# Add custom variables
custom_env.extend([
    ("WASM_MODE", "production"),
    ("MAX_MEMORY", "1073741824"),  # 1GB
    ("DEBUG_LEVEL", "info"),
    ("ALLOWED_HOSTS", "api.example.com,cdn.example.com")
])

# Configure WASI
wasi_config = wasmtime.WasiConfig()
wasi_config.env(custom_env)

# Alternative: inherit everything and let WebAssembly filter
# wasi_config_simple = wasmtime.WasiConfig()
# wasi_config_simple.inherit_env()

print(f"Configured {len(custom_env)} environment variables for WebAssembly")
print("Custom variables:")
for key, value in custom_env:
    if key.startswith(('WASM_', 'MAX_', 'DEBUG_', 'ALLOWED_')):
        print(f"  {key}={value}")
```

### Complete WASI Application Setup

```python
import wasmtime
import os
import sys
import tempfile
import shutil

def setup_wasi_application(wasm_path: str, work_dir: str = None):
    """
    Complete setup for a WASI WebAssembly application.
    
    Parameters:
    - wasm_path: Path to the WebAssembly WASI binary
    - work_dir: Working directory for the application (optional)
    """
    
    # Create working directory if not provided
    if work_dir is None:
        work_dir = tempfile.mkdtemp(prefix="wasi_app_")
        cleanup_work_dir = True
    else:
        cleanup_work_dir = False
    
    try:
        # Create application directory structure
        app_dirs = {
            "data": os.path.join(work_dir, "data"),
            "config": os.path.join(work_dir, "config"),
            "logs": os.path.join(work_dir, "logs"),
            "temp": os.path.join(work_dir, "temp")
        }
        
        for dir_path in app_dirs.values():
            os.makedirs(dir_path, exist_ok=True)
        
        # Create sample configuration
        config_content = '''
        {
            "app_name": "WASI Application",
            "version": "1.0.0",
            "max_memory": "512MB",
            "log_level": "info"
        }
        '''
        
        with open(os.path.join(app_dirs["config"], "app.json"), "w") as f:
            f.write(config_content)
        
        # Configure WASI
        wasi_config = wasmtime.WasiConfig()
        
        # Command line arguments
        wasi_config.argv([
            "wasi_app",
            "--config", "/config/app.json",
            "--data-dir", "/data",
            "--log-dir", "/logs"
        ])
        
        # Environment variables
        wasi_config.env([
            ("HOME", "/app"),
            ("TMPDIR", "/temp"),
            ("LOG_LEVEL", "info"),
            ("APP_VERSION", "1.0.0")
        ])
        
        # Filesystem access
        wasi_config.preopen_dir(app_dirs["data"], "/data", wasmtime.DirPerms.READ_WRITE)
        wasi_config.preopen_dir(app_dirs["config"], "/config", wasmtime.DirPerms.READ_ONLY)
        wasi_config.preopen_dir(app_dirs["logs"], "/logs", wasmtime.DirPerms.READ_WRITE)
        wasi_config.preopen_dir(app_dirs["temp"], "/temp", wasmtime.DirPerms.READ_WRITE)
        
        # I/O configuration
        wasi_config.inherit_stdin()
        wasi_config.inherit_stdout()
        wasi_config.inherit_stderr()
        
        # Set up WebAssembly runtime
        engine = wasmtime.Engine()
        store = wasmtime.Store(engine)
        linker = wasmtime.Linker(engine)
        
        # Define WASI imports
        linker.define_wasi(store, wasi_config)
        
        # Load the WebAssembly module
        module = wasmtime.Module.from_file(engine, wasm_path)
        
        # Instantiate the module
        instance = linker.instantiate(store, module)
        
        print(f"WASI application setup complete:")
        print(f"  Work directory: {work_dir}")
        print(f"  WebAssembly module: {wasm_path}")
        print(f"  Directory mappings:")
        for guest_path, host_path in [("/data", app_dirs["data"]),
                                     ("/config", app_dirs["config"]),
                                     ("/logs", app_dirs["logs"]),
                                     ("/temp", app_dirs["temp"])]:
            print(f"    {guest_path} -> {host_path}")
        
        # Run the application
        try:
            start_func = instance.get_export(store, "_start")
            if start_func:
                print("Running WASI application...")
                start_func(store)
                print("Application completed successfully")
            else:
                print("No _start function found in WebAssembly module")
                
        except wasmtime.ExitTrap as exit_trap:
            exit_code = exit_trap.code
            if exit_code == 0:
                print("Application completed successfully")
            else:
                print(f"Application exited with code: {exit_code}")
        
        except wasmtime.Trap as trap:
            print(f"Application trapped: {trap.message}")
            if trap.frames:
                for frame in trap.frames:
                    print(f"  at {frame.func_name or 'unknown'} in {frame.module_name or 'unknown'}")
        
        return instance
        
    finally:
        if cleanup_work_dir and os.path.exists(work_dir):
            shutil.rmtree(work_dir)

# Example usage:
# setup_wasi_application("my_wasi_app.wasm", "/path/to/work/directory")
```

### WASI Error Handling

```python
import wasmtime

def safe_wasi_execution(wasm_path: str):
    """Demonstrate proper error handling for WASI applications"""
    
    try:
        # Configure WASI
        wasi_config = wasmtime.WasiConfig()
        wasi_config.inherit_argv()
        wasi_config.inherit_env()
        wasi_config.inherit_stdin()
        wasi_config.inherit_stdout()
        wasi_config.inherit_stderr()
        
        # Set up runtime
        engine = wasmtime.Engine()
        store = wasmtime.Store(engine)
        linker = wasmtime.Linker(engine)
        linker.define_wasi(store, wasi_config)
        
        # Load module
        module = wasmtime.Module.from_file(engine, wasm_path)
        instance = linker.instantiate(store, module)
        
        # Execute
        start_func = instance.get_export(store, "_start")
        if start_func:
            start_func(store)
            
    except FileNotFoundError:
        print(f"Error: WebAssembly file '{wasm_path}' not found")
        
    except wasmtime.WasmtimeError as e:
        print(f"WebAssembly error: {e}")
        
    except wasmtime.ExitTrap as exit_trap:
        exit_code = exit_trap.code
        if exit_code != 0:
            print(f"Application exited with error code: {exit_code}")
        return exit_code
        
    except wasmtime.Trap as trap:
        print(f"Runtime trap: {trap.message}")
        print("Stack trace:")
        for frame in trap.frames:
            func_name = frame.func_name or f"func[{frame.func_index}]"
            module_name = frame.module_name or "unknown"
            print(f"  {func_name} in {module_name} at offset {frame.module_offset}")
        return 1
        
    except Exception as e:
        print(f"Unexpected error: {e}")
        return 1
        
    return 0
```