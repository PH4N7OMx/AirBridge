param([string]$Folder=(Split-Path (Split-Path $PSScriptRoot)))
$ErrorActionPreference='Stop'
$exe=Join-Path $Folder 'AirBridge.exe'
if (Get-Process AirBridge -ErrorAction SilentlyContinue | Where-Object Path -eq $exe) {throw 'Close the running AirBridge before starting this disruptive test.'}
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class RustStabilityNative {
 [DllImport("kernel32.dll")] public static extern IntPtr OpenProcess(uint access,bool inherit,int pid);
 [DllImport("ntdll.dll")] public static extern int NtSuspendProcess(IntPtr handle);
 [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr handle);
 [DllImport("user32.dll",CharSet=CharSet.Unicode,EntryPoint="FindWindowW")] public static extern IntPtr FindWindow(string className,string title);
 [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd,uint msg,UIntPtr wParam,IntPtr lParam);
}
'@
function Wait-Receiver([int]$Parent,[int]$Previous=0,[int]$Seconds=15) {
 $until=[DateTime]::UtcNow.AddSeconds($Seconds)
 do {
  $info=Get-CimInstance Win32_Process -Filter "Name='AirBridge.exe' AND ParentProcessId=$Parent" | Where-Object ExecutablePath -eq $exe
  foreach($item in $info) {
   $p=Get-Process -Id $item.ProcessId -ErrorAction SilentlyContinue
   if($p -and $p.Id -ne $Previous -and $p.MainWindowHandle -ne 0){return $p}
  }
  Start-Sleep -Milliseconds 150
 } while([DateTime]::UtcNow -lt $until)
 throw "Receiver did not appear within $Seconds seconds"
}
$supervisor=$null
try {
 $supervisor=Start-Process -FilePath $exe -WindowStyle Hidden -PassThru
 $receiver=Wait-Receiver $supervisor.Id
 'PASS: Rust supervisor starts Rust receiver window'
 $duplicate=Start-Process -FilePath $exe -WindowStyle Hidden -PassThru
 if(-not $duplicate.WaitForExit(3000)){throw 'Duplicate launcher remains running'}
 $children=@(Get-CimInstance Win32_Process -Filter "Name='AirBridge.exe' AND ParentProcessId=$($supervisor.Id)")
 if($children.Count -ne 1){throw 'Duplicate receiver'}
 'PASS: duplicate launch exits without creating another receiver'
 $old=$receiver.Id;$receiver.Kill();$receiver.WaitForExit(2000) | Out-Null
 $receiver=Wait-Receiver $supervisor.Id $old
 "PASS: crash recovery $old -> $($receiver.Id)"
 $old=$receiver.Id
 $handle=[RustStabilityNative]::OpenProcess(0x0800,$false,$old)
 if($handle -eq [IntPtr]::Zero){throw 'Could not open receiver for hang simulation'}
 try {if([RustStabilityNative]::NtSuspendProcess($handle) -ne 0){throw 'Could not suspend receiver'}} finally {[RustStabilityNative]::CloseHandle($handle)|Out-Null}
 $receiver=Wait-Receiver $supervisor.Id $old 45
 "PASS: hung-process recovery $old -> $($receiver.Id)"
 $old=$receiver.Id;$receiver.Kill();$receiver.WaitForExit(2000)|Out-Null
 Start-Sleep -Seconds 2
 $children=@(Get-CimInstance Win32_Process -Filter "Name='AirBridge.exe' AND ParentProcessId=$($supervisor.Id)")
 if($children.Count -ne 0 -or $supervisor.HasExited){throw 'Automatic recovery limit did not pause the receiver'}
 'PASS: repeated failure pauses automatic recovery while keeping the tray alive'
 $tray=[RustStabilityNative]::FindWindow('AirBridge.Rust.Supervisor','AirBridge')
 if($tray -eq [IntPtr]::Zero){throw 'Missing supervisor tray owner'}
 $null=[RustStabilityNative]::PostMessage($tray,0x111,[UIntPtr]102,[IntPtr]::Zero)
 $receiver=Wait-Receiver $supervisor.Id $old
 'PASS: manual reconnect clears the recovery limit'
 $owned=$receiver
 $supervisor.Kill();$supervisor.WaitForExit(3000)|Out-Null
 if(-not $owned.WaitForExit(5000)){throw 'Orphan receiver survived supervisor termination'}
 'PASS: killing the supervisor also terminates its receiver job'
 $supervisor=Start-Process -FilePath $exe -WindowStyle Hidden -PassThru
 $receiver=Wait-Receiver $supervisor.Id
 'PASS: restart after supervisor crash releases the instance guard'
 $null=$receiver.CloseMainWindow()
 if(-not $receiver.WaitForExit(10000)){throw 'Normal receiver close failed'}
 if(-not $supervisor.WaitForExit(5000)){throw 'Supervisor restarted a normally closed receiver'}
 'PASS: normal close exits both processes without restart'
 'All Rust lifecycle tests passed. Real iPhone playback is not covered.'
} finally {
 if($supervisor -and -not $supervisor.HasExited){$supervisor.Kill();$supervisor.WaitForExit(3000)|Out-Null}
}
