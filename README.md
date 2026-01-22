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

- https://github.com/ggml-org/whisper.cpp/releases
- https://huggingface.co/ggerganov/whisper.cpp/tree/main

```powershell
# Install ffmpeg
winget install --id=Gyan.FFmpeg -e

# Download whisper.cpp (whisper-bin-x64) from GitHub releases

# Extract the ZIP
Expand-Archive -Path "C:\\Users\\<User>\\Downloads\\whisper-bin-x64.zip" -DestinationPath "C:\\Users\\<User>\\Downloads"

# Create WhisperDesktop Directory
mkdir "C:\\Users\\<User>\\Documents\\WhisperDesktop\\"

# Place whisper.cpp (whisper-bin-x64)
mv "C:\\Users\\<User>\\Downloads\\whisper-bin-x64" "C:\\Users\\<User>\\Documents\\WhisperDesktop\\"

# Download models from Hugging Face

# Place the model
mv "C:\\Users\\<User>\\Downloads\\ggml-<MODEL>-v3.bin" "C:\\Users\\<User>\\Documents\\WhisperDesktop\\"

# Download the MSI installer from the Releases page

# Open the MSI installer and follow the setup wizard

# Launch WhisperDesktop from the Start Menu
```
