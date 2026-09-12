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

# ---- 1. build the anchor set from the WHOLE plan set --------------------------------
# The plan is split by area under docs/ but section numbers are GLOBAL across the set, so
# §4.5 resolves from docs/features.md no matter which file cites it. Two kinds of anchor:
#   headings    "## 5. Architecture" / "### 5.4 The API gateway"   -> 5, 5.4
#   list items  "3. **Autosave policy...**" under "## 10."         -> 10.3
# Code fences are skipped so shell blocks and startup-order snippets cannot invent anchors.
# Each anchor must have exactly ONE owner - that is what "never fork the plan" now means.
$validSections = @{}      # number -> list of files claiming it
$wp = Join-Path $Root 'whitepaper.md'
if (-not (Test-Path -LiteralPath $wp)) {
  Add-Finding $wp $null 'MISSING - the index must exist at the repo root'
}
$planSet = @()
if (Test-Path -LiteralPath $wp) { $planSet += Get-Item -LiteralPath $wp }
$planSet += @(Get-ChildItem -LiteralPath (Join-Path $Root 'docs') -File -Filter '*.md' -ErrorAction SilentlyContinue |
              Where-Object { $_.Name -ne 'README.md' })
foreach ($f in $planSet) {
  $top = $null
  $fence = $false
  foreach ($l in @(Get-Content -LiteralPath $f.FullName -Encoding UTF8)) {
    if ($l.TrimStart().StartsWith('```')) { $fence = -not $fence; continue }
    if ($fence) { continue }
    $claimed = $null
    if ($l -match '^##\s+(\d+)(?:\.\d+)?\.?\s') { $top = $matches[1]; $claimed = $top }
    elseif ($l -match '^###\s+(\d+(?:\.\d+)?)\.?\s') { $claimed = $matches[1] }
    if ($claimed) {
      if (-not $validSections.ContainsKey($claimed)) { $validSections[$claimed] = @() }
      $validSections[$claimed] += $f.Name
    }
    if ($top -and $l -match '^\s*(\d+)\.\s+\*?\*?[A-Za-z]') {
      $n = "$top.$($matches[1])"
      if (-not $validSections.ContainsKey($n)) { $validSections[$n] = @() }
      $validSections[$n] += $f.Name
    }
  }
}
foreach ($k in $validSections.Keys) {
  $owners = @($validSections[$k] | Select-Object -Unique)
  if ($owners.Count -gt 1) {
    Add-Finding (Join-Path $Root 'whitepaper.md') $null "section §$k is claimed by $($owners.Count) files ($($owners -join ', ')) - one owner per section, or the plan has forked"
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
  elseif ($rel -match '^docs/[a-z0-9-]+\.md$') { $ok = $true }   # plan areas, indexed by whitepaper.md
  elseif ($rel -match '^docs/') {
    Add-Finding $f.FullName $null 'unsupported location under docs/ - use docs/<area>.md, docs/dev/, or docs/decisions/'
  }
  if (-not $ok) { Add-Finding $f.FullName $null 'ORPHAN - no rule in the placement map covers this location' }

  # Intent has exactly one owner per numbered section. A second copy of a section is a fork
  # even when it has a different filename - checked against the anchor set built in step 1.
  if ($name -match '(?i)whitepaper' -and $rel -ne 'whitepaper.md') {
    Add-Finding $f.FullName $null 'filename claims to be the whitepaper; the index is whitepaper.md at the root and area docs must not reuse that name'
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

# ---- 2c. every plan area doc must declare what it owns -------------------------------
# `owns:` is what makes the split auditable: a file that does not say which sections it owns
# is a file whose content can silently drift into another area's remit.
foreach ($f in $planSet) {
  if ($f.Name -eq 'whitepaper.md') { continue }
  $fm = Get-FrontMatter $f
  if ($null -eq $fm) { Add-Finding $f.FullName $null 'plan area doc missing frontmatter (title, type, owns, status)'; continue }
  foreach ($k in @('title', 'type', 'owns', 'status')) {
    if (-not $fm.ContainsKey($k)) { Add-Finding $f.FullName $null "plan area doc missing frontmatter field '$k'" }
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
      if (-not $validSections.ContainsKey($m.Groups[1].Value)) {
        Add-Finding $f.FullName $n "dangling reference $SECT$($m.Groups[1].Value) - no owner anywhere in the plan set (renumbered? wrong document? use a path for skill sections)"
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
