# fix_staticdir.ps1
# Rewrite static_dir in manager.xml for the installed layout.
# install.bat calls this with two (or more) file paths as arguments.
# Paths are passed positionally (-File mode) so no cmd/PowerShell quoting
# gymnastics are needed around paths that may contain spaces or unicode.
#
# Example:
#   powershell -NoProfile -ExecutionPolicy Bypass -File "fix_staticdir.ps1" "D:\...\config\manager.xml" "D:\...\config\manager\manager.xml"
#
# Source config ships static_dir="../ui/dist" or "manager/ui/dist".
# The installed frontend lives in INSTALL_DIR\web, so rewrite to "web".

$q = [char]34
# static_dir="..." -> match the quoted value (any chars except a double quote)
$pat = 'static_dir=' + $q + '[' + '^' + $q + ']' + '+' + $q
$rep = 'static_dir=' + $q + 'web' + $q

foreach ($f in $args) {
    if (Test-Path $f) {
        (Get-Content $f) -replace $pat, $rep | Set-Content -Encoding UTF8 $f
        Write-Host "Fixed static_dir -> web : $f"
    } else {
        Write-Host "SKIP (not found)       : $f"
    }
}
