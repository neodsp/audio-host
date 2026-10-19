# Linux

## System dependencies for `rtaudio`

The `rtaudio` backend (enabled by default) builds the RtAudio C++ library from source, which
requires a C++ compiler, CMake, pkg-config, and the ALSA and PulseAudio development headers.

### Ubuntu/Debian

```sh
sudo apt update
sudo apt install cmake pkg-config libasound2-dev libpulse-dev
```

### Fedora

```sh
sudo dnf install cmake gcc-c++ pkgconf-pkg-config alsa-lib-devel pulseaudio-libs-devel
```

## System dependencies for `juce`

The `juce` backend builds JUCE from source with CMake. In addition to CMake, pkg-config, a C++
compiler, and the ALSA headers listed above, it requires the X11, FreeType, and Fontconfig
development headers.

### Ubuntu/Debian

```sh
sudo apt update
sudo apt install cmake pkg-config libasound2-dev libx11-dev libxext-dev libxrandr-dev \
  libxinerama-dev libxcursor-dev libxrender-dev libxcomposite-dev libfreetype-dev \
  libfontconfig1-dev
```

### Fedora

```sh
sudo dnf install cmake gcc-c++ pkgconf-pkg-config alsa-lib-devel libX11-devel libXext-devel \
  libXrandr-devel libXinerama-devel libXcursor-devel libXrender-devel libXcomposite-devel \
  freetype-devel fontconfig-devel
```
