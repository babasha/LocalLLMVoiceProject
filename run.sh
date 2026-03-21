#!/bin/bash
cd "$(dirname "$0")"
LD_LIBRARY_PATH=./libs/sherpa-onnx:$LD_LIBRARY_PATH \
  ./target/release/voice-translator --mode full "$@"
