$filtered = @($args | Where-Object { $_ -notlike '--target=*' })
& zig cc -target x86_64-linux-gnu @filtered
exit $LASTEXITCODE