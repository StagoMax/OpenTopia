@echo off
setlocal

rem Always start OpenTopia from the current workspace. pnpm starts the Vite
rem renderer and Electron rebuilds the Rust backend when its sources changed.
pushd "%~dp0.."
call pnpm.cmd --filter @opentopia/desktop dev
set "exit_code=%ERRORLEVEL%"
popd
exit /b %exit_code%
