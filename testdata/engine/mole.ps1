$items = @($args | ForEach-Object { $_ | ConvertTo-Json -Compress })
Write-Output ('{"invoked":[' + ($items -join ',') + ']}')

