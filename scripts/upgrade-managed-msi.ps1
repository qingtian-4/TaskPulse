# CI-only managed deployment. Do not change Windows Installer or UAC policy.
. (Join-Path $PSScriptRoot 'installer-process.ps1')

function Get-UpgradeInstallerPolicy {
    $result = [ordered]@{}
    foreach ($path in @('HKLM:\Software\Policies\Microsoft\Windows\Installer', 'HKCU:\Software\Policies\Microsoft\Windows\Installer', 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Policies\System')) {
        $values = Get-ItemProperty -LiteralPath $path -ErrorAction SilentlyContinue
        foreach ($name in @('DisableMSI','AlwaysInstallElevated','EnableLUA','ConsentPromptBehaviorAdmin','ConsentPromptBehaviorUser','FilterAdministratorToken')) {
            $result[$path+'\'+$name] = if ($values -and $values.PSObject.Properties[$name]) { $values.$name } else { $null }
        }
    }
    return $result | ConvertTo-Json -Compress
}

function Assert-UpgradePackageHash([string]$Path, [string]$Expected) {
    if ($Expected -notmatch '^[a-fA-F0-9]{64}$' -or (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash -ne $Expected) {
        throw 'Managed MSI package hash mismatch'
    }
}

function Initialize-ManagedMsiFixture($Info, [string]$Root, [string]$Evidence) {
    if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $Info.kind -ne 'msi') { throw 'Managed MSI fixture requires hosted Windows MSI acceptance' }
    if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'MSI authorization requires the CI administrator' }
    $protected = Join-Path $Root 'administrator-packages'
    if (Test-Path -LiteralPath $protected) { throw 'Managed package directory must be new' }
    New-Item -ItemType Directory -Path $protected | Out-Null
    $acl = Get-Acl -LiteralPath $protected
    $acl.SetAccessRuleProtection($true,$false)
    foreach ($sid in @('S-1-5-18','S-1-5-32-544')) {
        $rule = [Security.AccessControl.FileSystemAccessRule]::new([Security.Principal.SecurityIdentifier]::new($sid),'FullControl','ContainerInherit,ObjectInherit','None','Allow')
        $acl.AddAccessRule($rule)
    }
    Set-Acl -LiteralPath $protected -AclObject $acl
    $metadata = @{}
    $installer = New-Object -ComObject WindowsInstaller.Installer
    foreach ($label in @('old','new')) {
        $source = (Resolve-Path -LiteralPath $Info.($label+'_payload')).Path
        if (-not $source.StartsWith($Root+'\',[StringComparison]::OrdinalIgnoreCase)) { throw 'MSI source escapes fixture' }
        $package = Join-Path $protected ($label+'.msi')
        Copy-Item -LiteralPath $source -Destination $package
        Assert-UpgradePackageHash $package $Info.($label+'_sha256')
        $database = $installer.OpenDatabase($package,0)
        $properties = @{}
        foreach ($name in @('ProductCode','UpgradeCode','ProductVersion','ALLUSERS')) {
            $view = $database.OpenView("SELECT Value FROM Property WHERE Property='$name'")
            $view.Execute(); $row = $view.Fetch()
            if ($row) { $properties[$name] = $row.StringData(1) }
            $view.Close()
        }
        $expected = Get-Content -Raw -LiteralPath (Join-Path $Evidence ($label+'-msi-metadata.json')) | ConvertFrom-Json
        foreach ($name in @('ProductCode','UpgradeCode','ProductVersion','ALLUSERS')) {
            if ($properties[$name] -ne $expected.properties.$name) { throw 'MSI metadata mismatch' }
        }
        if ($properties.ALLUSERS -ne '1') { throw 'Expected per-machine MSI' }
        $metadata[$label] = $properties
    }
    if ($metadata.old.UpgradeCode -ne $metadata.new.UpgradeCode -or $metadata.old.ProductCode -eq $metadata.new.ProductCode) { throw 'Invalid major-upgrade identity pair' }
    # Keep Windows PowerShell 5.1's ANSI script decoding out of this fixture.
    $unicodeLeaf = ([char]0x6D4B).ToString()+[char]0x8BD5+' install'
    $install = Join-Path (Join-Path $Root 'Applications') $unicodeLeaf
    if (Test-Path -LiteralPath $install) { throw 'MSI installation directory must be new' }
    $null = New-Item -ItemType Directory -Path $install
    $msiexec = Join-Path $env:SystemRoot 'System32\msiexec.exe'
    $oldLog = Join-Path $Evidence 'msi-initial.log'
    $result = Invoke-InstallerProcess -FilePath $msiexec -Arguments ('/i "'+(Join-Path $protected 'old.msi')+'" /qn /norestart /L*v "'+$oldLog+'" INSTALLDIR="'+$install+'"') -TimeoutMilliseconds 180000
    if ($result.ExitCode -ne 0) { throw "Initial MSI install failed: $($result.ExitCode)" }
    $exe = Join-Path $install 'agent-tray.exe'
    Assert-UpgradePackageHash $exe $Info.old_exe_sha256
    if ($installer.ProductState($metadata.old.ProductCode) -ne 5) { throw 'Old MSI must be installed' }
    $advertiseLog = Join-Path $Evidence 'msi-advertise.log'
    # /jm authorizes this exact package. It must not install the new payload.
    $result = Invoke-InstallerProcess -FilePath $msiexec -Arguments ('/jm "'+(Join-Path $protected 'new.msi')+'" /qn /L*v "'+$advertiseLog+'"') -TimeoutMilliseconds 180000
    if ($result.ExitCode -ne 0) { throw "MSI advertisement failed: $($result.ExitCode)" }
    Assert-UpgradePackageHash $exe $Info.old_exe_sha256
    $oldState = $installer.ProductState($metadata.old.ProductCode)
    $newState = $installer.ProductState($metadata.new.ProductCode)
    if ($oldState -ne 5 -or $newState -ne 1) { throw "Advertisement must leave N installed and N+1 only advertised: $oldState/$newState" }
    @{mode='managed-msi-standard-user';old_product_state=$oldState;new_product_state=$newState;old_exe_unchanged=$true;packages_hash_checked=$true;uac_interaction_verified=$false} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Evidence 'msi-authorization.json') -Encoding UTF8
    $Info | Add-Member -NotePropertyName installed_exe -NotePropertyValue $exe -Force
    $Info | Add-Member -NotePropertyName msi_deployment -NotePropertyValue 'administrator-advertised-package' -Force
}
