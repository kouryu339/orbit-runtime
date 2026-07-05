$filtered = @($args | Where-Object { $_ -notlike '--target=*' })
& zig c++ -target x86_64-linux-gnu @filtered
exit $LASTEXITCODE