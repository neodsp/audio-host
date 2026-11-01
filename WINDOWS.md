# Windows

## ASIO with the `cpal-asio` feature

The `cpal` backend uses WASAPI on Windows by default. The `cpal-asio` feature adds cpal's ASIO
host, which talks to your audio interface's ASIO driver directly. These steps follow
[cpal's ASIO instructions](https://github.com/RustAudio/cpal#compiling-for-asio).

### 1. Prepare the build environment

1. **Install LLVM/Clang.** cpal generates its bindings to the ASIO SDK with `bindgen`, which
   needs clang. Install the pre-built LLVM binaries from <https://releases.llvm.org/download.html>,
   or run `winget install LLVM.LLVM`.
2. **Set `LIBCLANG_PATH`** to LLVM's `bin` directory. For the default install location:

   ```bat
   setx LIBCLANG_PATH "C:\Program Files\LLVM\bin"
   ```

   Open a new terminal afterwards, so the variable is picked up.
3. **Install Visual Studio** (or the Build Tools) with the C++ workload. The build script finds
   `vcvarsall.bat` and runs it for the right architecture. If that fails, run it yourself first,
   e.g.:

   ```bat
   "C:\Program Files (x86)\Microsoft Visual Studio\2019\Community\VC\Auxiliary\Build\vcvarsall.bat" amd64
   ```

4. **Install an ASIO driver.** Audio interfaces ship their own. To test without one, install
   [ASIO4ALL](https://www.asio4all.org/) and enable its "offline" option during installation.

### 2. The ASIO SDK

You normally don't have to do anything here. The build script looks for Steinberg's ASIO SDK in
this order:

1. The directory in the `CPAL_ASIO_DIR` environment variable, if it is set.
2. `asio_sdk` in the temporary directory (`%TEMP%\asio_sdk`), if an earlier build put it there.
3. Otherwise it downloads the SDK from <https://www.steinberg.net/asiosdk> into that directory.

Set `CPAL_ASIO_DIR` to an SDK you downloaded yourself if the build machine is offline, or to make
builds reproducible.

The ASIO SDK has its own license from Steinberg. Check its terms before you distribute an
application built with ASIO.

### 3. Enable the feature

```toml
[dependencies]
audio-host = { version = "0.2.0", default-features = false, features = ["cpal-asio"] }
```

The feature does nothing on other platforms, so the same `Cargo.toml` still builds on Linux and
macOS.

### 4. Select ASIO in your code

WASAPI stays the default host, so switch to ASIO before selecting devices and starting the stream:

```rust,no_run
use audio_host::{AudioBackend, AudioHost, Config, Error};

fn main() -> Result<(), Error> {
    let mut host = AudioHost::new()?;
    // `apis()` lists "ASIO" when the feature is enabled
    host.set_api("ASIO")?;
    println!("ASIO devices: {:#?}", host.outputs());

    host.start(Config::default(), |_input, _output| {})?;
    std::thread::sleep(std::time::Duration::from_secs(1));
    host.stop()
}
```

If ASIO can't be used (e.g. no ASIO driver is installed), `set_api("ASIO")` returns
`Error::NotFound` or `Error::Backend` and the current host stays selected.

### Cross-compiling from Linux or macOS

cpal can also build Windows applications with ASIO on Linux and macOS with the MinGW-w64 toolchain
(`x86_64-pc-windows-gnu`). Add the MinGW-w64 and LLVM include directories to `CPLUS_INCLUDE_PATH`,
see [cpal's cross-compilation notes](https://github.com/RustAudio/cpal#cross-compilation) for an
example.
