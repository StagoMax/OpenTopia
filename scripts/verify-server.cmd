@echo off
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0verify-server.ps1" %*
