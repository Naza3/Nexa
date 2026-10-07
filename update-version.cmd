@echo off
setlocal
py -3 -c "import sys; sys.exit(sys.version_info < (3, 11))" >nul 2>&1
if errorlevel 1 goto use_python
py -3 "%~dp0scripts\set_version.py" %*
goto done

:use_python
python -c "import sys; sys.exit(sys.version_info < (3, 11))" >nul 2>&1
if errorlevel 1 goto missing_python
python "%~dp0scripts\set_version.py" %*
goto done

:missing_python
echo Python 3.11 or newer is required. Install Python and run this tool again.
if "%~1"=="" pause
exit /b 1

:done
set "nexa_version_exit=%errorlevel%"
if "%~1"=="" pause
exit /b %nexa_version_exit%
