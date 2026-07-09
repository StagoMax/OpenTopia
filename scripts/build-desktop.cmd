@echo off
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0build-desktop.ps1" %*
