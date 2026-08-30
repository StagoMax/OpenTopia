@echo off
setlocal

rem Always start OpenTopia from the current workspace. pnpm starts the Vite
rem renderer and Electron rebuilds the Rust backend when its sources changed.
rem Pin the desktop build cache to an isolated I: directory so shortcut
rem launches neither depend on Explorer's environment nor contend with tests.
set "OPENTOPIA_DEV_CARGO_TARGET_DIR=I:\BuildCache\OpenTopia\desktop-dev"

pushd "%~dp0.."
call pnpm.cmd --filter @opentopia/desktop dev
set "exit_code=%ERRORLEVEL%"
popd
exit /b %exit_code%
