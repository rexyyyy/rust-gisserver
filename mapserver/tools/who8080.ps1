$conn = Get-NetTCPConnection -LocalPort 8080 -State Listen | Select-Object -First 1
$p = Get-CimInstance Win32_Process -Filter "ProcessId=$($conn.OwningProcess)"
"Pid=$($p.ProcessId)"
"Exe=$($p.ExecutablePath)"
"Cmd=$($p.CommandLine)"
