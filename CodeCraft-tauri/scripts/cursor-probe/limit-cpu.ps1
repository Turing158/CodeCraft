$cursorProbeProcess = [System.Diagnostics.Process]::GetCurrentProcess()
$cursorProbeAllowedMask = $cursorProbeProcess.ProcessorAffinity.ToInt64()
$cursorProbeMask = [long]0
$cursorProbeCores = 0
for ($cursorProbeBit = 0; $cursorProbeBit -lt 64 -and $cursorProbeCores -lt 2; $cursorProbeBit++) {
    $cursorProbeBitMask = [long]1 -shl $cursorProbeBit
    if (($cursorProbeAllowedMask -band $cursorProbeBitMask) -ne 0) {
        $cursorProbeMask = $cursorProbeMask -bor $cursorProbeBitMask
        $cursorProbeCores++
    }
}
if ($cursorProbeCores -eq 0) { throw 'No allowed logical CPU cores' }
$cursorProbeProcess.ProcessorAffinity = [IntPtr]$cursorProbeMask
$env:CARGO_BUILD_JOBS = '2'
$env:RUST_TEST_THREADS = '2'
