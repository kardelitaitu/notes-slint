# check-docs.ps1 -- documentation placement validator.
# Part of the doc-management skill. Exits 1 if anything is misplaced or inconsistent.
#
# Usage:  pwsh .agents/skills/doc-management/scripts/check-docs.ps1 [-Root <path>]
#
# ASCII only on purpose: Windows PowerShell 5.1 misreads non-ASCII script literals
# when the file has no BOM, so the section-sign character is built from its codepoint.

[CmdletBinding()]
param(
  # default: four levels up from scripts/ -> repo root
  [string]$Root = (Split-Path -Parent (Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))))
)

$ErrorActionPreference = 'Stop'
$SECT = [string][char]0x00A7          # the section-sign character
$Statuses = @('proposed', 'implemented', 'rejected', 'archived')
$RootDocs = @('README.md', 'AGENTS.md', 'whitepaper.md')
$NoteIdPattern  = '^\d{4}-\d{2}-\d{2}-[a-z0-9]+(-[a-z0-9]+){1,5}\.md$'
$AdrIdPattern   = '^\d{4}-[a-z0-9]+(-[a-z0-9]+)*\.md$'
$RequiredFields = @('title', 'status', 'id', 'created', 'updated', 'relates')

$findings = New-Object System.Collections.Generic.List[string]
function Add-Finding($path, $lineNo, $msg) {
  $rel = $path.Substring([Math]::Min($Root.Length, $path.Length)).TrimStart('\', '/')
  if ($lineNo) { $findings.Add("$rel`:$lineNo  $msg") } else { $findings.Add("$rel  $msg") }
}

if (-not (Test-Path -LiteralPath $Root)) { Write-Error "root not found: $Root" }
$Root = (Resolve-Path -LiteralPath $Root).Path

$all = Get-ChildItem -LiteralPath $Root -Recurse -File -Filter '*.md' | Where-Object {
  $_.FullName -notmatch '\\(target|node_modules|\.git|dist|build)\\'
}

function Get-FrontMatter($file) {
  $lines = @(Get-Content -LiteralPath $file.FullName -Encoding UTF8)
  if ($lines.Count -lt 1 -or $lines[0].Trim() -ne '---') { return $null }
  $end = -1
  for ($i = 1; $i -lt $lines.Count; $i++) { if ($lines[$i].Trim() -eq '---') { $end = $i; break } }
  if ($end -lt 0) { return $null }
  $map = @{}
  for ($i = 1; $i -lt $end; $i++) {
    if ($lines[$i] -match '^([A-Za-z_][A-Za-z0-9_]*)\s*:\s*(.*)$') { $map[$matches[1]] = $matches[2].Trim() }
  }
  return $map
}

# ---- 1. build the set of valid section numbers from whitepaper.md -----------------
# Two kinds of addressable anchor:
#   headings    "## 5. Architecture" / "### 5.4 The API gateway"   -> 5, 5.4
#   list items  "3. **Autosave policy...**" under "## 10."         -> 10.3
# The second kind matters because decisions are enumerated as a list, and refs like
# §10.3 are how the whole document points at them. Code fences are skipped, so shell
# blocks and startup-order snippets cannot invent anchors.
$validSections = New-Object System.Collections.Generic.HashSet[string]
$wp = Join-Path $Root 'whitepaper.md'
if (-not (Test-Path -LiteralPath $wp)) {
  Add-Finding $wp $null 'MISSING - the founding sketch must exist at the repo root'
} else {
  $top = $null
  $fence = $false
  foreach ($l in @(Get-Content -LiteralPath $wp -Encoding UTF8)) {
    if ($l.TrimStart().StartsWith('```')) { $fence = -not $fence; continue }
    if ($fence) { continue }
    if ($l -match '^##\s+(\d+)(?:\.\d+)?\.?\s') {
      $top = $matches[1]
      [void]$validSections.Add($top)
    }
    elseif ($l -match '^###\s+(\d+(?:\.\d+)?)\.?\s') {
      [void]$validSections.Add($matches[1])
    }
    if ($top -and $l -match '^\s*(\d+)\.\s+\*?\*?[A-Za-z]') {
      [void]$validSections.Add("$top.$($matches[1])")
    }
  }
}

# ---- 2. every markdown file must sit in a sanctioned location ---------------------
foreach ($f in $all) {
  $rel = $f.FullName.Substring($Root.Length).TrimStart('\', '/') -replace '\\', '/'
  $dir = Split-Path -Parent $rel
  if ($dir -eq '') { $dir = '.' }
  $name = $f.Name

  $ok = $false
  if ($dir -eq '.') {
    if ($RootDocs -contains $name) { $ok = $true }
    else { Add-Finding $f.FullName $null "root-level '$name' is not one of: $($RootDocs -join ', '). Add a row to the placement map instead of a new root file." }
  }
  elseif ($rel -match '^\.agents/notes/(proposed|implemented|rejected|archived)/') { $ok = $true }
  elseif ($rel -match '^\.agents/notes/README\.md$') { $ok = $true }
  elseif ($rel -match '^\.agents/skills/[^/]+/') { $ok = $true }
  elseif ($rel -match '^docs/(decisions|dev)/') { $ok = $true }
  elseif ($rel -match '^docs/README\.md$') { $ok = $true }
  elseif ($rel -match '^docs/[^/]+\.md$') {
    Add-Finding $f.FullName $null 'loose file in docs/ - that root holds only README.md; a new subfolder needs a row in the placement map first'
  }
  elseif ($rel -match '^docs/') { $ok = $true }
  if (-not $ok) { Add-Finding $f.FullName $null 'ORPHAN - no rule in the placement map covers this location' }

  # Intent has exactly one home. Two copies of a product definition guarantee that someone,
  # eventually, reads the wrong one.
  if ($name -match '(?i)whitepaper|roadmap' -and $rel -ne 'whitepaper.md') {
    Add-Finding $f.FullName $null 'appears to fork whitepaper.md - there is exactly one. Edit it, or open a note in .agents/notes/proposed/.'
  }
}

# ---- 2b. a plan is not a dev doc ---------------------------------------------------
# docs/dev/ describes code that exists. Its cheapest honest proxy: there is a Cargo.toml.
# Writing "getting-started.md" before the first build works produces a confident
# description of something that may never be built - and it reads with authority anyway.
if (-not (Test-Path -LiteralPath (Join-Path $Root 'Cargo.toml'))) {
  $devDocs = $all | Where-Object {
    ($_.FullName -replace '\\', '/') -match '/docs/dev/[^/]+\.md$' -and $_.Name -ne 'README.md'
  }
  foreach ($f in $devDocs) {
    Add-Finding $f.FullName $null "premature dev doc - docs/dev/ describes code that exists, and there is no Cargo.toml yet. Keep this in whitepaper.md or a proposed/ note until the code lands."
  }
}

# ---- 3. notes: filename, status/folder agreement, frontmatter, ids ----------------
$ids = @{}
$notes = $all | Where-Object { $_.FullName -replace '\\', '/' -match '/\.agents/notes/(proposed|implemented|rejected|archived)/[^/]+\.md$' }
foreach ($f in $notes) {
  $rel = $f.FullName -replace '\\', '/'
  if ($f.Name -eq 'README.md') { continue }

  if ($f.Name -notmatch $NoteIdPattern) {
    Add-Finding $f.FullName $null "filename '$($f.Name)' must be YYYY-MM-DD-short-kebab-slug.md"
  }

  $folder = ($rel -replace '.*/notes/([a-z]+)/.*$', '$1')
  $fm = Get-FrontMatter $f
  if ($null -eq $fm) { Add-Finding $f.FullName $null 'missing or malformed frontmatter block'; continue }

  foreach ($k in $RequiredFields) {
    if (-not $fm.ContainsKey($k) -or [string]::IsNullOrWhiteSpace($fm[$k])) {
      Add-Finding $f.FullName $null "frontmatter field '$k' is missing or empty"
    }
  }

  if ($fm.ContainsKey('status')) {
    if ($Statuses -notcontains $fm['status']) {
      Add-Finding $f.FullName $null "status '$($fm['status'])' is not one of: $($Statuses -join ', ')"
    }
    elseif ($fm['status'] -ne $folder) {
      Add-Finding $f.FullName $null "status '$($fm['status'])' does not match folder '$folder/' - a stale status will be cited as fact"
    }
  }

  if ($fm.ContainsKey('created') -and $fm['created'] -notmatch '^\d{4}-\d{2}-\d{2}$') {
    Add-Finding $f.FullName $null "created '$($fm['created'])' is not YYYY-MM-DD"
  }
  if ($fm.ContainsKey('id') -and $fm['id'] -notmatch '^\d{4}-\d{2}-\d{2}-[a-z0-9-]+$') {
    Add-Finding $f.FullName $null "id '$($fm['id'])' must look like YYYY-MM-DD-slug"
  }
  if ($fm.ContainsKey('id')) {
    if ($ids.ContainsKey($fm['id'])) { Add-Finding $f.FullName $null "duplicate id '$($fm['id'])' (also in $($ids[$fm['id']]))" }
    else { $ids[$fm['id']] = $rel }
  }

  $body = @(Get-Content -LiteralPath $f.FullName -Encoding UTF8)
  $text = $body -join "`n"
  if ($text -match '(?m)^##\s*Recommendation' ) { } else {
    Add-Finding $f.FullName $null 'no "## Recommendation" section - a note without a recommendation is a ticket, not a note'
  }
  if ($folder -eq 'rejected' -and $text -notmatch '(?m)^##\s*Reopening') {
    Add-Finding $f.FullName $null 'rejected note has no "## Reopening conditions" - a rejection without a re-opening condition is a grudge, not a decision'
  }
  if ($folder -eq 'archived' -and (-not $fm.ContainsKey('decision') -or $fm['decision'] -in @('null', ''))) {
    Add-Finding $f.FullName $null 'archived note has no decision: pointer - say what superseded it'
  }
}

# ---- 4. ADRs ----------------------------------------------------------------------
$adrDir = Join-Path $Root 'docs/decisions'
if (Test-Path -LiteralPath $adrDir) {
  foreach ($f in @(Get-ChildItem -LiteralPath $adrDir -File -Filter '*.md' | Where-Object { $_.Name -ne 'README.md' })) {
    if ($f.Name -notmatch $AdrIdPattern) {
      Add-Finding $f.FullName $null 'ADR filename must be NNNN-short-kebab-slug.md (4-digit, zero-padded)'
    }
    $fm = Get-FrontMatter $f
    if ($null -eq $fm) { Add-Finding $f.FullName $null 'ADR missing frontmatter'; continue }
    if (-not $fm['status'] -or $fm['status'] -notmatch '^(proposed|accepted|deprecated|superseded)$') {
      Add-Finding $f.FullName $null "ADR status '$($fm['status'])' invalid (ADR statuses are a separate vocabulary from note statuses)"
    }
    if ($fm['status'] -eq 'superseded' -and (-not $fm.ContainsKey('superseded_by') -or $fm['superseded_by'] -eq 'null')) {
      Add-Finding $f.FullName $null 'superseded ADR must name superseded_by:'
    }
  }
}

# ---- 5. section references must resolve -------------------------------------------
$skipRefCheck = @('/.agents/skills/doc-management/templates/', '/.agents/skills/doc-management/scripts/')
foreach ($f in $all) {
  $rel = $f.FullName -replace '\\', '/'
  foreach ($s in $skipRefCheck) { if ($rel -like "*$s*") { $skip = $true } }
  if ($skip) { $skip = $false; continue }
  $n = 0
  foreach ($l in @(Get-Content -LiteralPath $f.FullName -Encoding UTF8)) {
    $n++
    foreach ($m in [regex]::Matches($l, "$SECT(\d+(?:\.\d+)?)")) {
      if (-not $validSections.Contains($m.Groups[1].Value)) {
        Add-Finding $f.FullName $n "dangling reference $SECT$($m.Groups[1].Value) - no such section in whitepaper.md (renumbered? wrong document? use a path for skill sections)"
      }
    }
  }
}

# ---- report -----------------------------------------------------------------------
Write-Host "check-docs: scanned $($all.Count) markdown files under $Root"
if ($findings.Count -eq 0) {
  Write-Host 'check-docs: clean' -ForegroundColor Green
  exit 0
}
$findings | ForEach-Object { Write-Host "  $_" -ForegroundColor Yellow }
Write-Host "check-docs: $($findings.Count) problem(s)" -ForegroundColor Red
exit 1
