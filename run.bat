@echo off
set "mingwBin=%LOCALAPPDATA%\Microsoft\WinGet\Packages\BrechtSanders.WinLibs.MCF.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64\bin"
if exist "%mingwBin%" set "PATH=%mingwBin%;%PATH%"
cargo run --release
