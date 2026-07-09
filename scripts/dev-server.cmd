@echo off
powershell -NoProfile -ExecutionPolicy Bypass -Command ". '%~dp0dev-env.ps1'; Set-Location '%~dp0..'; cargo run -p opentopia-server -- %*"
