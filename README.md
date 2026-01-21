<div align="right">

![GitHub Actions Workflow Status](https://img.shields.io/github/actions/workflow/status/Arata1202/WhisperDesktop/publish.yml)
![GitHub License](https://img.shields.io/github/license/Arata1202/WhisperDesktop)

</div>

## Getting Started

### Install on macOS

```bash

# Install dependencies
brew install whisper-cpp
brew install ffmpeg

# Download the DMG file from the Releases page

# Open the DMG file and drag whisperdesktop.app into the Applications folder

# Remove the macOS quarantine attribute
xattr -d com.apple.quarantine "/Applications/whisperdesktop.app"

# Launch WhisperDesktop from the Applications folder

```

### Install on Windows

```bash
# Install dependencies
winget install --id ggerganov.whisper.cpp --accept-source-agreements --accept-package-agreements
winget install --id Gyan.FFmpeg --accept-source-agreements --accept-package-agreements

# Download the MSI installer from the Releases page

# Open the MSI installer and follow the setup wizard

# Launch WhisperDesktop from the Start Menu
```
