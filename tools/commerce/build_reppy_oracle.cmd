@echo off
setlocal
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" >nul
set "REPPY_SRC=research\commerce\donors\checkouts\seomoz--reppy\reppy\rep-cpp"
cl /nologo /EHsc /std:c++14 /I"%REPPY_SRC%\include" /I"%REPPY_SRC%\deps\url-cpp\include" tools\commerce\reppy_oracle.cpp "%REPPY_SRC%\src\agent.cpp" "%REPPY_SRC%\src\directive.cpp" "%REPPY_SRC%\src\robots.cpp" "%REPPY_SRC%\deps\url-cpp\src\url.cpp" "%REPPY_SRC%\deps\url-cpp\src\utf8.cpp" "%REPPY_SRC%\deps\url-cpp\src\punycode.cpp" "%REPPY_SRC%\deps\url-cpp\src\psl.cpp" /Fo:target\ /Fe:target\reppy-oracle.exe
