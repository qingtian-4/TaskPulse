#!/bin/bash
# TaskPulse public-release bootstrap. macOS system tools only; Bash 3.2 compatible.
set -euo pipefail
language=en
case "${LANG:-}" in zh*) language=zh-CN;; esac
version= prerelease=0 download_only=0 output_dir=. install_dir="${HOME}/Applications" no_launch=0
work= stage= lock= recovery= target= attached=0 transaction=0 committed=0 incoming_id= old_id= finished=0
version_pattern='^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-(alpha|beta|rc)\.(0|[1-9][0-9]*))?$'
tag_pattern="^v${version_pattern#^}"
time_pattern='^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$'
commit_pattern='^[a-f0-9]{40}$'
digest_pattern='^[a-f0-9]{64}$'
executable_pattern='^[A-Za-z0-9_-]+$'
say() { if [ "$language" = zh-CN ]; then printf '%s\n' "$2"; else printf '%s\n' "$1"; fi; }
fail() { say "Error: $1" "错误：${2:-$1}" >&2; exit 1; }
usage() {
  say 'TaskPulse macOS installer: --version VERSION --prerelease --download-only --output-dir DIR --dir DIR --no-launch --language en|zh-CN --help' \
      'TaskPulse macOS 安装：--version 版本 --prerelease --download-only --output-dir 目录 --dir 目录 --no-launch --language en|zh-CN --help'
}
# Fixed JSON paths only. Never evaluate JSON as shell code.
field() { plutil -extract "$2" raw -o - "$1" 2>/dev/null; }
required() { local value; value=$(field "$1" "$2") || fail "Missing JSON field: $2" "JSON 缺少字段：$2"; printf '%s' "$value"; }
exists() { [ -e "$1" ] || [ -L "$1" ]; }
directory_id() { [ ! -L "$1" ] && stat -f '%d:%i' "$1"; }
# BSD mv may nest into an externally created directory. Track filesystem identity
# before/after moving; never move a colliding foreign child into our cleanup tree.
move_owned() {
  local source=$1 destination=$2 expected nested
  expected=$(directory_id "$source") || return 1
  exists "$destination" && return 1
  mv -n "$source" "$destination" || return 1
  if exists "$source"; then return 1; fi
  if [ "$(directory_id "$destination" 2>/dev/null || true)" = "$expected" ]; then return 0; fi
  nested="$destination/${source##*/}"
  if [ "$(directory_id "$nested" 2>/dev/null || true)" = "$expected" ]; then
    # Source is now absent, so the owned directory can be restored for cleanup.
    mv -n "$nested" "$source" || return 1
  fi
  return 1
}
cleanup() {
  local code=$? retain=0
  # Older Bash can enter EXIT after a parse error with status zero.
  if [ "$code" = 0 ] && [ "$finished" = 0 ]; then code=1; fi
  trap - EXIT INT TERM
  if [ "$transaction" = 1 ] && [ "$committed" = 0 ]; then
    if exists "$target"; then
      if [ -n "$incoming_id" ] && [ "$(directory_id "$target" 2>/dev/null || true)" = "$incoming_id" ]; then
        move_owned "$target" "$stage/failed.app" || retain=1
      elif [ -n "$old_id" ] && [ "$(directory_id "$target" 2>/dev/null || true)" = "$old_id" ]; then
        : # Old app was never moved; keep it in place.
      else retain=1; fi
    fi
    if [ -d "$recovery/TaskPulse.app" ]; then
      if ! exists "$target" && move_owned "$recovery/TaskPulse.app" "$target"; then
        say 'Previous app restored.' '已恢复旧版应用。'
        rmdir "$recovery" || retain=1
      else retain=1; fi
    elif [ -d "$recovery" ]; then
      rmdir "$recovery" || retain=1
    fi
    if [ "$retain" = 1 ]; then
      say "Recovery required: $recovery (staging: $stage). Inspect before retrying." "需要恢复：${recovery}（暂存：${stage}）。核对后再重试。" >&2
    fi
  fi
  if [ "$attached" = 1 ]; then
    if ! hdiutil detach "$work/mount" >"$work/detach.log" 2>&1; then
      say "Could not detach owned mount; retained: $work" "无法卸载本次挂载，已保留：$work" >&2
      retain=1; code=1
    fi
  fi
  if [ "$retain" = 0 ]; then
    [ -z "$stage" ] || rm -rf "$stage"
    [ -z "$work" ] || rm -rf "$work"
  fi
  if [ -n "$lock" ]; then rmdir "$lock" || code=1; fi
  if [ "$code" != 0 ]; then
    if [ "$committed" = 1 ]; then
      say 'TaskPulse is installed, but startup or cleanup needs attention. See https://support.apple.com/en-us/102445' \
          'TaskPulse 已安装，但启动或收尾仍需处理。请参阅 https://support.apple.com/en-us/102445' >&2
    else
      say 'Installation did not complete. No security attributes were removed. For macOS launch alerts see https://support.apple.com/en-us/102445' \
          '安装未完成，未移除安全属性。macOS 启动提示请查阅 https://support.apple.com/en-us/102445' >&2
    fi
  fi
  exit "$code"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
while [ "$#" -gt 0 ]; do
  case "$1" in
    --prerelease) prerelease=1; shift;;
    --download-only) download_only=1; shift;;
    --no-launch) no_launch=1; shift;;
    --help) usage; finished=1; exit 0;;
    --version|--output-dir|--dir|--language)
      [ "$#" -ge 2 ] && [ -n "$2" ] && [[ "$2" != --* ]] || fail "Missing value: $1" "参数缺少值：$1"
      case "$1" in --version) version=${2#v};; --output-dir) output_dir=$2;; --dir) install_dir=$2;; --language) language=$2;; esac
      shift 2;;
    *) fail "Unknown argument: $1" "未知参数：$1";;
  esac
done
case "$language" in en|zh-CN) ;; *) fail 'Language must be en or zh-CN.' '语言必须为 en 或 zh-CN。';; esac
[ -z "$version" ] || [[ "$version" =~ $version_pattern ]] || fail 'Invalid version.' '版本无效。'
[ "$(uname -s)" = Darwin ] || fail 'This installer requires macOS.' '此脚本仅支持 macOS。'
machine=$(uname -m)
case "$machine" in
  arm64) arch=arm64; binary_arch=arm64;;
  x86_64)
    # Apple documents ENOENT for sysctl.proc_translated as native execution.
    # Read the diagnostic in C locale so missing keys differ from denied reads.
    translated_ok=1
    translated=$(LC_ALL=C sysctl -n sysctl.proc_translated 2>&1) || translated_ok=0
    arm=$(sysctl -in hw.optional.arm64 2>/dev/null || true)
    if [ "$translated" = 1 ] || [ "$arm" = 1 ]; then arch=arm64; binary_arch=arm64
    elif [ "$translated_ok" = 1 ] && [ "$translated" = 0 ]; then arch=x64; binary_arch=x86_64
    elif [ "$translated_ok" = 0 ] && [[ "$translated" == *"unknown oid"*"sysctl.proc_translated"* ]]; then arch=x64; binary_arch=x86_64
    else fail 'Cannot determine physical architecture.' '无法确定真实硬件架构。'; fi;;
  *) fail 'Unsupported architecture.' '不支持此架构。';;
esac
work=$(mktemp -d "${TMPDIR:-/tmp}/taskpulse-download.XXXXXX")
fetch() {
  curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' --tlsv1.2 \
    --connect-timeout 15 --max-time "$3" --retry 2 --retry-max-time 30 \
    --header 'Accept: application/vnd.github+json' --user-agent TaskPulse-Installer --output "$2" "$1"
}
api=https://api.github.com/repos/qingtian-4/TaskPulse/releases
index=
if [ -n "$version" ]; then fetch "$api/tags/v$version" "$work/release.json" 45
elif [ "$prerelease" = 0 ]; then fetch "$api/latest" "$work/release.json" 45
else
  fetch "$api?per_page=100" "$work/releases.json" 45
  newest=
  for ((i=0; i<100; i++)); do
    field "$work/releases.json" "$i.tag_name" >/dev/null || break
    draft=$(required "$work/releases.json" "$i.draft")
    published=$(field "$work/releases.json" "$i.published_at" || true)
    if [ "$draft" = false ] && [[ "$published" =~ $time_pattern ]] && [[ "$published" > "$newest" ]]; then newest=$published; index=$i; fi
  done
  [ -n "$index" ] || fail 'No public release available. Drafts cannot be installed.' '无可用公开版本，不能安装草稿。'
  plutil -extract "$index" json -o "$work/release.json" "$work/releases.json"
fi
release="$work/release.json"
[ "$(required "$release" draft)" = false ] || fail 'Drafts cannot be installed.' '不能安装草稿。'
published=$(required "$release" published_at)
[[ "$published" =~ $time_pattern ]] || fail 'Release is not published.' '版本尚未公开。'
tag=$(required "$release" tag_name)
[[ "$tag" =~ $tag_pattern ]] || fail 'Invalid release tag.' '版本标签无效。'
if [ -n "$version" ]; then [ "$tag" = "v$version" ] || fail 'Release tag mismatch.' '版本标签不匹配。'
elif [ "$prerelease" = 0 ]; then
  [ "$(required "$release" prerelease)" = false ] && [[ "$tag" != *-* ]] || fail 'Prerelease requires explicit selection.' '预发布须明确选择。'
fi
version=${tag#v}
name="TaskPulse_${version}_macos_${arch}.dmg"
sums="macos-${arch}-SHA256SUMS.txt"
manifest="macos-${arch}-manifest.json"
# Check every location before transferring any asset. Duplicate names fail closed.
urls=()
for wanted in "$name" "$sums" "$manifest"; do
  count=0; url=
  for ((i=0; i<1000; i++)); do
    candidate=$(field "$release" "assets.$i.name") || break
    if [ "$candidate" = "$wanted" ]; then
      count=$((count+1)); url=$(required "$release" "assets.$i.browser_download_url")
    fi
  done
  [ "$count" = 1 ] && [ "$url" = "https://github.com/qingtian-4/TaskPulse/releases/download/$tag/$wanted" ] || fail "Missing, duplicate or unexpected asset: $wanted" "资产缺失、重复或来源错误：$wanted"
  urls[${#urls[@]}]=$url
done
if [ "$download_only" = 1 ]; then
  mkdir -p "$output_dir"
  output_dir=$(cd "$output_dir" && pwd -P)
  destination="$output_dir/TaskPulse-$tag-macos-$arch"
  exists "$destination" && fail "Destination exists: $destination" "目标已存在：$destination"
  lock_candidate="$output_dir/.TaskPulse-$tag-macos-$arch.lock"
  mkdir "$lock_candidate" 2>/dev/null || fail "Download locked: $lock_candidate" "下载已锁定：$lock_candidate"
  lock=$lock_candidate
  stage=$(mktemp -d "$output_dir/.taskpulse-download.XXXXXX")
else stage="$work/assets"; mkdir "$stage"; fi
say "Downloading TaskPulse $tag ($arch)" "正在下载 TaskPulse ${tag}（${arch}）"
# curl does not attach browser download quarantine. Add it without replacing
# any existing value, so CLI downloads receive the system's security assessment.
quarantine() {
  local value
  if ! value=$(xattr -p com.apple.quarantine "$1" 2>/dev/null); then
    value="0083;$(printf '%x' "$(date +%s)");TaskPulse;$(uuidgen)"
    xattr -w com.apple.quarantine "$value" "$1"
  fi
  [ -n "$value" ] && [ "$(xattr -p com.apple.quarantine "$1")" = "$value" ] || fail 'Cannot preserve download quarantine.' '无法保留下载安全属性。'
}
fetch "${urls[0]}" "$stage/$name" 300
quarantine "$stage/$name"
fetch "${urls[1]}" "$stage/$sums" 45
fetch "${urls[2]}" "$stage/$manifest" 45
metadata="$stage/$manifest"
[ "$(required "$metadata" version)" = "$version" ] && [ "$(required "$metadata" platform)" = macos ] && [ "$(required "$metadata" architecture)" = "$arch" ] || fail 'Manifest version/platform/architecture mismatch.' 'manifest 版本、平台或架构不匹配。'
[[ "$(required "$metadata" commit)" =~ $commit_pattern ]] || fail 'Invalid source SHA.' '源码 SHA 无效。'
[ "$(required "$metadata" assets.0)" = "$name" ] && ! field "$metadata" assets.1 >/dev/null || fail 'Unexpected manifest assets.' 'manifest 资产列表不符。'
# Older public experimental manifests have no tag/build_kind; explicit preview/candidate markers are rejected.
kind=$(field "$metadata" build_kind || true)
[ -z "$kind" ] || [ "$kind" = release ] || fail 'Preview/candidate assets are not public releases.' '预览或候选附件不是公开 Release。'
if field "$metadata" tag >/dev/null; then [ "$(field "$metadata" tag)" = "$tag" ] || fail 'Manifest tag mismatch.' 'manifest 标签不匹配。'; fi
expected=$(required "$metadata" sha256)
[[ "$expected" =~ $digest_pattern ]] || fail 'Invalid SHA-256.' 'SHA-256 无效。'
[ "$(cat "$stage/$sums")" = "$expected  $name" ] || fail 'Checksum manifest mismatch.' '校验文件不匹配。'
actual=$(shasum -a 256 "$stage/$name"); actual=${actual%% *}
[ "$actual" = "$expected" ] || fail 'SHA-256 mismatch.' 'SHA-256 不匹配。'
say 'SHA-256 verified.' 'SHA-256 校验通过。'
if [ "$download_only" = 1 ]; then
  # The lock serializes bootstrap writers. Detect BSD mv nesting if an
  # external writer creates the destination during publication, and undo it.
  exists "$destination" && fail "Destination exists: $destination" "目标已存在：$destination"
  move_owned "$stage" "$destination" || fail 'Download publication conflict; external data preserved.' '保存下载发生冲突，已保留外部数据。'
  stage=
  say "Verified download saved: $destination" "已保存验证后的下载：$destination"
  finished=1
  exit 0
fi
mkdir -p "$install_dir"
install_dir=$(cd "$install_dir" && pwd -P)
target="$install_dir/TaskPulse.app"
recovery="$install_dir/.taskpulse-recovery"
lock_candidate="$install_dir/.taskpulse-install.lock"
mkdir "$lock_candidate" 2>/dev/null || fail "Install locked: $lock_candidate" "安装已锁定：$lock_candidate"
lock=$lock_candidate
exists "$recovery" && fail "Recovery directory requires inspection: $recovery" "须先核对恢复目录：$recovery"
# Validate product identity before any replacement or process inspection.
identity() {
  [ -d "$1" ] && [ ! -L "$1" ] && [ ! -L "$1/Contents" ] && [ ! -L "$1/Contents/Info.plist" ] || fail 'Refusing a symlink or invalid application.' '拒绝符号链接或无效应用。'
  [ "$(required "$1/Contents/Info.plist" CFBundleIdentifier)" = app.taskpulse.desktop ] || fail 'Target belongs to another product.' '目标属于其他产品。'
}
running() {
  local listing pid command
  listing=$(ps -axww -o pid= -o comm=) || fail 'Cannot inspect running applications.' '无法检查运行中的应用。'
  while read -r pid command; do
    case "$command" in "$target/Contents/MacOS/"*) return 0;; esac
  done <<< "$listing"
  return 1
}
os_version_number() {
  local major minor patch
  [[ "$1" =~ ^(0|[1-9][0-9]{0,2})\.(0|[1-9][0-9]{0,2})(\.(0|[1-9][0-9]{0,2}))?$ ]] || return 1
  IFS=. read -r major minor patch <<< "$1"
  printf '%s' "$((major * 1000000 + minor * 1000 + ${patch:-0}))"
}
system_version=$(sw_vers -productVersion) || fail 'Cannot read macOS version.' '无法读取 macOS 版本。'
system_version_number=$(os_version_number "$system_version") || fail 'Cannot validate macOS version.' '无法验证 macOS 版本。'
verify_app() {
  local app=$1 exe bundle_version architectures minimum minimum_number
  identity "$app"
  minimum=$(required "$app/Contents/Info.plist" LSMinimumSystemVersion)
  minimum_number=$(os_version_number "$minimum") || fail 'Invalid minimum macOS version.' 'macOS 最低版本信息无效。'
  [ "$system_version_number" -ge "$minimum_number" ] || fail "TaskPulse requires macOS $minimum or later; installed macOS is $system_version." "TaskPulse 需要 macOS ${minimum} 或更高版本；当前系统为 ${system_version}。"
  bundle_version=$(required "$app/Contents/Info.plist" CFBundleShortVersionString)
  [ "$bundle_version" = "$version" ] || fail 'Application version mismatch.' '应用版本不匹配。'
  exe=$(required "$app/Contents/Info.plist" CFBundleExecutable)
  [[ "$exe" =~ $executable_pattern ]] && [ -f "$app/Contents/MacOS/$exe" ] && [ ! -L "$app/Contents/MacOS" ] && [ ! -L "$app/Contents/MacOS/$exe" ] || fail 'Invalid bundle executable.' '应用可执行文件无效。'
  # Read the thin Mach-O CPU type with built-in tools, without requiring Xcode.
  architectures=$(od -An -tx1 -N8 "$app/Contents/MacOS/$exe" | tr -d '[:space:]')
  case "$binary_arch:$architectures" in
    arm64:cffaedfe0c000001|x86_64:cffaedfe07000001) ;;
    *) fail 'Binary architecture mismatch.' '实际二进制架构不匹配。';;
  esac
  codesign --verify --deep --strict "$app"
}
if exists "$target"; then
  identity "$target"
  running && fail 'Quit the installed TaskPulse normally, then retry.' '请正常退出已安装的 TaskPulse 后重试。'
fi
mkdir "$work/mount"
attached=1
hdiutil attach "$stage/$name" -readonly -nobrowse -mountpoint "$work/mount" >"$work/attach.log" 2>&1
shopt -s nullglob dotglob
apps=("$work/mount/"*.app)
[ "${#apps[@]}" = 1 ] && [ "${apps[0]}" = "$work/mount/TaskPulse.app" ] || fail 'DMG must contain only the expected TaskPulse app.' 'DMG 必须仅含预期的 TaskPulse 应用。'
verify_app "${apps[0]}"
# Target staging must be on the same filesystem as the installed app.
app_stage=$(mktemp -d "$install_dir/.taskpulse-stage.XXXXXX")
stage=$app_stage
ditto "${apps[0]}" "$stage/TaskPulse.app"
quarantine "$stage/TaskPulse.app"
verify_app "$stage/TaskPulse.app"
hdiutil detach "$work/mount" >"$work/detach.log" 2>&1
attached=0
if exists "$target"; then
  identity "$target"
  running && fail 'TaskPulse started during installation; quit and retry.' '安装期间 TaskPulse 已启动，请退出后重试。'
  if [ "$(required "$target/Contents/Info.plist" CFBundleShortVersionString)" = "$version" ]; then
    verify_app "$target"
    say "Already installed: $version" "已安装此版本：$version"
    finished=1
    exit 0
  fi
  old_id=$(directory_id "$target")
  mkdir "$recovery"
  transaction=1
  move_owned "$target" "$recovery/TaskPulse.app" || fail 'Could not preserve previous app.' '无法保留旧版应用。' 
else transaction=1; fi
incoming_id=$(directory_id "$stage/TaskPulse.app")
move_owned "$stage/TaskPulse.app" "$target" || fail 'Application publication conflict.' '应用安装目录发生冲突。' 
verify_app "$target"
quarantine "$target"
say 'Signature integrity verified. Packages use ad-hoc signing without notarization. Gatekeeper is preserved.' \
    '签名完整性通过。安装包为 ad-hoc 签名且未公证，保留 Gatekeeper。'
# Installation is committed only after bundle, OS, signature and quarantine checks.
# LaunchServices may reject a valid installation until the user reviews macOS alerts.
committed=1
say "Installed TaskPulse $version: $target" "已安装 TaskPulse ${version}：$target"
if [ "$no_launch" = 0 ]; then
  if ! open "$target" >"$work/launch.log" 2>&1; then
    cat "$work/launch.log" >&2
    if [ -n "$old_id" ]; then say "Previous app retained: $recovery/TaskPulse.app. Inspect before retrying." "旧版保留于：${recovery}/TaskPulse.app。请核对后再重试。" >&2; fi
    fail "Could not launch the installed app: $target. Review the original error and macOS Privacy & Security settings." "无法启动已安装的应用：${target}。请核对原始错误及 macOS 隐私与安全性设置。"
  fi
fi
if [ -d "$recovery" ]; then
  if [ -n "$old_id" ] && [ "$(directory_id "$recovery/TaskPulse.app" 2>/dev/null || true)" = "$old_id" ]; then
    rm -rf "$recovery/TaskPulse.app"
    rmdir "$recovery" || fail "Inspect retained recovery directory: $recovery" "请核对残留恢复目录：$recovery"
  else fail "Recovery ownership changed: $recovery" "恢复目录所有权发生变化：$recovery"; fi
fi
finished=1
