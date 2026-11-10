# CI-only managed deployment. Do not change Windows Installer or UAC policy.
. (Join-Path $PSScriptRoot '.\installer-process.ps1')

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

function Protect-UpgradeDirectory([string]$Directory) {
    $acl = Get-Acl -LiteralPath $Directory
    $acl.SetAccessRuleProtection($true,$false)
    foreach ($sid in @('S-1-5-18','S-1-5-32-544')) {
        $rule = [Security.AccessControl.FileSystemAccessRule]::new([Security.Principal.SecurityIdentifier]::new($sid),'FullControl','ContainerInherit,ObjectInherit','None','Allow')
        $acl.AddAccessRule($rule)
    }
    Set-Acl -LiteralPath $Directory -AclObject $acl
}

function Initialize-ManagedMsiFixture($Info, [string]$Root, [string]$Evidence) {
    if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $Info.kind -ne 'msi') { throw 'Managed MSI fixture requires hosted Windows MSI acceptance' }
    if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'MSI authorization requires the CI administrator' }
    $protected = Join-Path $Root 'administrator-packages'
    if (Test-Path -LiteralPath $protected) { throw 'Managed package directory must be new' }
    New-Item -ItemType Directory -Path $protected | Out-Null
    Protect-UpgradeDirectory $protected
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
    $applications = Join-Path $Root 'Applications'
    New-Item -ItemType Directory -Path $applications | Out-Null
    Protect-UpgradeDirectory $applications
    $install = Join-Path $applications $unicodeLeaf
    if (Test-Path -LiteralPath $install) { throw 'MSI installation directory must be new' }
    $null = New-Item -ItemType Directory -Path $install
    $msiexec = Join-Path $env:SystemRoot 'System32\msiexec.exe'
    $oldLog = Join-Path $Evidence 'msi-initial.log'
    $result = Invoke-InstallerProcess -FilePath $msiexec -Arguments ('/i "'+(Join-Path $protected 'old.msi')+'" /qn /norestart /L*v "'+$oldLog+'" INSTALLDIR="'+$install+'"') -TimeoutMilliseconds 180000
    if ($result.ExitCode -ne 0) { throw "Initial MSI install failed: $($result.ExitCode)" }
    if ($Info.old_exe_name -notin @('agent-tray.exe', 'TaskPulse.exe')) { throw 'Unrecognized old executable name' }
    $exe = Join-Path $install $Info.old_exe_name
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
    $approved = @{product_code=$metadata.new.ProductCode;old_product_code=$metadata.old.ProductCode;sha256=$Info.new_sha256} | ConvertTo-Json -Compress
    [IO.File]::WriteAllText((Join-Path $protected 'approved-update.json'), $approved, [Text.UTF8Encoding]::new($false))
    @{mode='managed-msi-standard-user';old_product_state=$oldState;new_product_state=$newState;old_exe_unchanged=$true;packages_hash_checked=$true;uac_interaction_verified=$false} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Evidence 'msi-authorization.json') -Encoding UTF8
    $Info | Add-Member -NotePropertyName installed_exe -NotePropertyValue $exe -Force
    $Info | Add-Member -NotePropertyName msi_deployment -NotePropertyValue 'administrator-advertised-package' -Force
}


function Invoke-ApprovedMsiServicing($Info, [string]$Root, [string]$Evidence, $Dispatch) {
    if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $Info.kind -ne 'msi') { throw 'MSI servicing requires a disposable hosted runner' }
    $protected = Join-Path $Root 'administrator-packages'
    $approval = Get-Content -Raw -LiteralPath (Join-Path $protected 'approved-update.json') | ConvertFrom-Json
    if ($Dispatch.route -ne 'approved-msi-servicing' -or $Dispatch.download_matches_protected_package -ne $true -or
        $Dispatch.sha256 -ne $Info.new_sha256 -or $Dispatch.sha256 -ne $approval.sha256 -or
        $Dispatch.product_code -ne $approval.product_code -or $approval.product_code -notmatch '^\{[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}\}$') {
        throw 'The standard-account request does not match the administrator-approved update'
    }
    $package = Join-Path $protected 'new.msi'
    Assert-UpgradePackageHash $package $Info.new_sha256
    Assert-UpgradePackageHash $Info.installed_exe $Info.old_exe_sha256
    $installer = New-Object -ComObject WindowsInstaller.Installer
    if ($installer.ProductState($approval.product_code) -ne 1 -or
        $installer.ProductState($approval.old_product_code) -ne 5) {
        throw 'Expected the approved replacement to be advertised and the old MSI installed'
    }
    # Install only the administrator-protected package. Do not auto-launch
    # from the elevated installer; the original standard account owns launch.
    $install = Split-Path -Parent $Info.installed_exe
    $msiexec = Join-Path $env:SystemRoot 'System32\msiexec.exe'
    $log = Join-Path $protected 'msi-approved-servicing.log'
    $parameters = '/i "'+$package+'" /qn /norestart /L*v "'+$log+'" INSTALLDIR="'+$install+'"'
    $result = Invoke-InstallerProcess -FilePath $msiexec -Arguments $parameters -TimeoutMilliseconds 180000
    if ($result.ExitCode -ne 0) { throw "Approved MSI servicing failed: $($result.ExitCode)" }
    if ($Info.new_exe_name -ne 'TaskPulse.exe') { throw 'Unexpected renamed executable' }
    Assert-UpgradePackageHash (Join-Path $install $Info.new_exe_name) $Info.new_exe_sha256
    if ($installer.ProductState($approval.product_code) -ne 5 -or $installer.ProductState($approval.old_product_code) -eq 5) {
        throw 'Windows Installer did not replace the original product'
    }
    $receipt = @{route='administrator-approved-servicing';sha256=$approval.sha256;product_code=$approval.product_code;installed_exe_sha256=$Info.new_exe_sha256}
    $json = $receipt | ConvertTo-Json -Compress
    # Publish only a complete receipt, and keep it protected from the app account.
    $pending = Join-Path $protected 'managed-msi-installed.pending'
    [IO.File]::WriteAllText($pending, $json, [Text.UTF8Encoding]::new($false))
    Move-Item -LiteralPath $pending -Destination (Join-Path $protected 'managed-msi-installed.json')
}
