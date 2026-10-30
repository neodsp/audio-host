# Linux

## System dependencies for `cpal`

### Ubuntu/Debian

```sh
sudo apt update
sudo apt install pkg-config libasound2-dev
```

### Fedora

```sh
sudo dnf install pkgconf-pkg-config alsa-lib-devel
```

## System dependencies for `rtaudio`

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

### Ubuntu/Debian

```sh
sudo apt update
sudo apt install cmake pkg-config libasound2-dev libx11-dev libxext-dev libxrandr-dev \
  libxinerama-dev libxcursor-dev libxrender-dev libxcomposite-dev libfreetype-dev \
  libfontconfig1-dev libjack-jackd2-dev
```

### Fedora

```sh
sudo dnf install cmake gcc-c++ pkgconf-pkg-config alsa-lib-devel libX11-devel libXext-devel \
  libXrandr-devel libXinerama-devel libXcursor-devel libXrender-devel libXcomposite-devel \
  freetype-devel fontconfig-devel jack-audio-connection-kit-devel
```
