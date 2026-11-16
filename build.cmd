@echo off
rem Build KlikCore and put KlikCore.exe in this folder (double-click it to open the launcher).
setlocal
cd /d "%~dp0"
where cargo >nul 2>nul || set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
cargo build --release || goto :failed
rem A running KlikCore.exe can't be overwritten, but it can be moved aside;
rem KlikCore deletes KlikCore.old.exe the next time it starts.
if exist KlikCore.exe (
    del /q KlikCore.old.exe 2>nul
    move /y KlikCore.exe KlikCore.old.exe >nul
)
copy /y target\release\klikcore.exe KlikCore.exe >nul || goto :failed
echo.
echo Built KlikCore.exe - double-click it to open the launcher.
exit /b 0
:failed
echo.
echo Build failed.
pause
exit /b 1
