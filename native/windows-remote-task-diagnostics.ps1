# Diagnostic evidence for this isolated Windows task process only.
function Enable-WindowsRemoteTaskDump {
    param([string]$Executable, [string]$LogDirectory)
    $folder = Join-Path $LogDirectory 'crash-dumps'
    [void][IO.Directory]::CreateDirectory($folder)
    $key = 'HKLM:\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\' + [IO.Path]::GetFileName($Executable)
    if (Test-Path -LiteralPath $key) { throw "Unexpected existing task-specific dump policy: $key" }
    [void](New-Item -Path $key -Force)
    [void](New-ItemProperty -LiteralPath $key -Name DumpFolder -Value $folder -PropertyType ExpandString)
    [void](New-ItemProperty -LiteralPath $key -Name DumpCount -Value 2 -PropertyType DWord)
    [void](New-ItemProperty -LiteralPath $key -Name DumpType -Value 1 -PropertyType DWord)
    return $key
}

function Save-WindowsRemoteTaskExit {
    param([int]$Code, [int]$ProcessId, [datetime]$Started, [string]$Label, [string]$LogDirectory, [bool]$Fixture)
    $unsigned = [BitConverter]::ToUInt32([BitConverter]::GetBytes($Code), 0)
    $hex = '0x{0:X8}' -f $unsigned
    [ordered]@{ code = $Code; hex = $hex; pid = $ProcessId; started = $Started.ToUniversalTime().ToString('o') } |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $LogDirectory "$Label.exit.json")
    if ($Fixture -and $Code -ne 0) {
        Write-Host "$Label exited with $Code ($hex), PID $ProcessId."
        $filter = @{
            LogName = 'Application'; Id = @(1000, 1001); StartTime = $Started
        }
        $events = @(Get-WinEvent -FilterHashtable $filter -MaxEvents 20 -ErrorAction SilentlyContinue |
            Select-Object -Property @('TimeCreated', 'Id', 'ProviderName', 'Message'))
        ConvertTo-Json -InputObject $events -Depth 4 |
            Set-Content -LiteralPath (Join-Path $LogDirectory "$Label.windows-events.json")
    }
}
