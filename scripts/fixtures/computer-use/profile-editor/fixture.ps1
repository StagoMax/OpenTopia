param(
  [Parameter(Mandatory = $true)][string]$StatePath
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
[System.Windows.Forms.Application]::EnableVisualStyles()

$form = New-Object System.Windows.Forms.Form
$form.Text = "OpenTopia Computer Use - Profile Editor"
$form.ClientSize = New-Object System.Drawing.Size(620, 390)
$form.FormBorderStyle = [System.Windows.Forms.FormBorderStyle]::FixedDialog
$form.MaximizeBox = $false
$form.MinimizeBox = $false
$form.StartPosition = [System.Windows.Forms.FormStartPosition]::CenterScreen
$form.Font = New-Object System.Drawing.Font("Segoe UI", 10)

$heading = New-Object System.Windows.Forms.Label
$heading.Text = "Profile editor"
$heading.Font = New-Object System.Drawing.Font("Segoe UI", 16, [System.Drawing.FontStyle]::Bold)
$heading.AutoSize = $true
$heading.Location = New-Object System.Drawing.Point(28, 24)

$instruction = New-Object System.Windows.Forms.Label
$instruction.Text = "Configure the profile, then save it."
$instruction.AutoSize = $true
$instruction.Location = New-Object System.Drawing.Point(31, 63)

$nameLabel = New-Object System.Windows.Forms.Label
$nameLabel.Text = "Workspace name"
$nameLabel.AutoSize = $true
$nameLabel.Location = New-Object System.Drawing.Point(31, 112)

$name = New-Object System.Windows.Forms.TextBox
$name.Name = "workspaceName"
$name.Size = New-Object System.Drawing.Size(385, 30)
$name.Location = New-Object System.Drawing.Point(180, 108)

$modeLabel = New-Object System.Windows.Forms.Label
$modeLabel.Text = "Operation mode"
$modeLabel.AutoSize = $true
$modeLabel.Location = New-Object System.Drawing.Point(31, 163)

$mode = New-Object System.Windows.Forms.ComboBox
$mode.Name = "operationMode"
$mode.DropDownStyle = [System.Windows.Forms.ComboBoxStyle]::DropDownList
$mode.Size = New-Object System.Drawing.Size(220, 30)
$mode.Location = New-Object System.Drawing.Point(180, 159)
[void]$mode.Items.AddRange([string[]]@("Focused", "Balanced", "Precision"))
$mode.SelectedIndex = 0

$history = New-Object System.Windows.Forms.CheckBox
$history.Name = "keepLocalHistory"
$history.Text = "Keep local history"
$history.AutoSize = $true
$history.Location = New-Object System.Drawing.Point(180, 213)

$save = New-Object System.Windows.Forms.Button
$save.Name = "saveProfile"
$save.Text = "Save profile"
$save.Size = New-Object System.Drawing.Size(140, 38)
$save.Location = New-Object System.Drawing.Point(425, 314)

$status = New-Object System.Windows.Forms.Label
$status.Name = "saveStatus"
$status.Text = "Unsaved changes"
$status.AutoSize = $true
$status.Location = New-Object System.Drawing.Point(31, 326)

$save.Add_Click({
  $state = [ordered]@{
    schemaVersion = 1
    workspaceName = $name.Text
    operationMode = [string]$mode.SelectedItem
    keepLocalHistory = $history.Checked
    savedAt = [DateTime]::UtcNow.ToString("o")
  }
  $json = $state | ConvertTo-Json -Depth 4
  [IO.File]::WriteAllText($StatePath, "$json`n", [Text.UTF8Encoding]::new($false))
  $status.Text = "Profile saved"
})

[void]$form.Controls.AddRange(@(
  $heading,
  $instruction,
  $nameLabel,
  $name,
  $modeLabel,
  $mode,
  $history,
  $save,
  $status
))
[void]$form.ShowDialog()
