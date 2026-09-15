@echo off
rem preview.bat - double-click launcher for the Slint product build: target\debug\notes-slint.exe
rem STATE: launched this way the product writes its state to the folder "target\debug\data" beside the exe, NOT to %APPDATA%\notes-gpui, because crates/bridge-slint/src/product.rs:156-166 resolves the dir from std::env::current_exe(), and a "data" directory beside the exe (the portable form) wins over the roaming profile; it never looks at the working directory. Remove that data dir and the next launch falls back to %APPDATA%\notes-gpui.

setlocal
rem Works from a double-click or any cwd. Also the folder cargo has to run from.
cd /d "%~dp0"
set "EXE=%~dp0target\debug\notes-slint.exe"
set "DATA=%~dp0target\debug\data"

if exist "%EXE%" goto run
echo [preview] no exe at %EXE%
echo [preview] building it once: cargo build -p notes-bridge-slint --bin notes-slint
cargo build -p notes-bridge-slint --bin notes-slint
if errorlevel 1 goto buildfailed
if not exist "%EXE%" goto notafter
:run

for %%F in ("%EXE%") do set "EXE_ABS=%%~fF"
for %%F in ("%EXE%") do set "STAMP=%%~tF"
if exist "%DATA%\" (set "STATE=%DATA%") else (set "STATE=%APPDATA%\notes-gpui")

echo [preview] exe          : %EXE_ABS%
echo [preview] last written : %STAMP%
echo [preview] state dir    : %STATE%  - this launch writes session.json and notes\untitled.notes there; it follows the EXE, not the folder of this console.
echo [preview] second copy  : double-clicking this again opens ANOTHER window over that same session.json and untitled.notes, last writer wins - there is no single-instance guard.
echo [preview] launching detached - closing this console leaves the app running.

start "" "%EXE_ABS%"
exit /b 0

:buildfailed
echo [preview] cargo build FAILED - nothing was launched. Read the cargo error above, fix it, then run this again.
exit /b 1

:notafter
echo [preview] cargo build reported success but %EXE% is still missing - nothing was launched.
exit /b 1
