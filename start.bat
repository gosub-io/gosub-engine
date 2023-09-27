@echo off
echo Compiling GoSub...
cargo build --release

if %errorlevel% neq 0 (
  echo Compilation failed.
  pause
  exit /b %errorlevel%
)

echo Starting GoSub...


pause
