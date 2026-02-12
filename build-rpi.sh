#!/bin/bash
# Build script for Raspberry Pi 3 Model B
# Target: ARM Cortex-A53 (aarch64-unknown-linux-gnu)

set -e

echo "=== RustClaw Raspberry Pi 3 Build Script ==="
echo ""

# Check if we're on macOS or Linux
if [[ "$OSTYPE" == "darwin"* ]]; then
    echo "Detected: macOS"
    CROSS_COMPILE=true
elif [[ "$OSTYPE" == "linux-gnu"* ]]; then
    echo "Detected: Linux"
    # Check if we're on Raspberry Pi
    if [[ $(uname -m) == "aarch64" ]] || [[ $(uname -m) == "armv7l" ]]; then
        echo "Building natively on Raspberry Pi"
        CROSS_COMPILE=false
    else
        echo "Cross-compiling from Linux x86_64 to ARM"
        CROSS_COMPILE=true
    fi
else
    echo "Unknown OS: $OSTYPE"
    exit 1
fi

if [ "$CROSS_COMPILE" = true ]; then
    echo ""
    echo "=== Setting up cross-compilation ==="
    
    # Install cross if not present
    if ! command -v cross &> /dev/null; then
        echo "Installing cross..."
        cargo install cross --git https://github.com/cross-rs/cross
    fi
    
    # Build for ARM64 (Raspberry Pi 3 with 64-bit OS)
    echo "Building for aarch64-unknown-linux-gnu..."
    cross build --target aarch64-unknown-linux-gnu --release
    
    echo ""
    echo "=== Build complete ==="
    echo "Binary location: target/aarch64-unknown-linux-gnu/release/rustclaw"
    echo ""
    echo "To deploy to Raspberry Pi:"
    echo "  scp target/aarch64-unknown-linux-gnu/release/rustclaw pi@raspberrypi.local:~/"
    echo "  ssh pi@raspberrypi.local 'chmod +x ~/rustclaw'"
    
else
    echo ""
    echo "=== Building natively on Raspberry Pi ==="
    
    # Ensure we have enough swap
    echo "Checking swap space..."
    SWAP_SIZE=$(free -m | awk '/Swap:/ {print $2}')
    if [ "$SWAP_SIZE" -lt 1024 ]; then
        echo "Warning: Swap is less than 1GB. Build may fail."
        echo "Recommended: sudo dphys-swapfile swapoff && sudo nano /etc/dphys-swapfile"
        echo "Set CONF_SWAPSIZE=1024, then: sudo dphys-swapfile setup && sudo dphys-swapfile swapon"
        read -p "Continue anyway? (y/N) " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            exit 1
        fi
    fi
    
    # Build with single thread to save RAM
    echo "Building with single thread to save RAM..."
    cargo build --release --jobs 1
    
    echo ""
    echo "=== Build complete ==="
    echo "Binary location: target/release/rustclaw"
    echo "Binary size: $(ls -lh target/release/rustclaw | awk '{print $5}')"
fi

echo ""
echo "=== Next steps ==="
echo "1. Copy binary to Raspberry Pi"
echo "2. Set up environment variables (see README.md)"
echo "3. Run: ./rustclaw --mode telegram"
echo ""
