# CoShell

SSH 접속, 포트 터널, AI 도우미를 한곳에서 쓰는 터미널입니다. Rust와 [iced](https://iced.rs)로 만들었습니다.

- **서버 관리** — `ssh` 명령을 붙여넣어 등록, 그룹·환경(운영/스테이징/개발) 태그, 경유 서버(ProxyJump). `~/.ssh/config`의 서버도 함께 보입니다.
- **운영 서버 보호** — 운영 세션은 빨간 프레임으로 표시하고, 터미널 출력을 AI로 보내기 전에 한 번 더 확인합니다.
- **터널** — 목적별(서버 포트 열기 / 내 PC 포트 공유 / SOCKS 프록시)로 포트 포워딩을 열고 상태를 봅니다.
- **AI 도우미** — Anthropic·OpenAI 호환 API. 비밀 값은 마스킹하고, 제안된 명령은 위험도를 표시한 뒤 입력만 합니다(실행은 직접).
- **터미널** — 텍스트 선택·복사, 출력 검색, 마우스 지원(vim·tmux), 링크 열기, 색 구성·글꼴·커서 설정.

## 빌드와 실행

Rust(edition 2024)가 필요합니다.

```bash
cargo run -p coshell-app                 # 로컬 셸로 시작
cargo run -p coshell-app -- ssh my-host   # ~/.ssh/config 또는 등록한 서버로 접속
cargo test --workspace
```

macOS 앱 번들과 DMG:

```bash
scripts/bundle-macos.sh    # target/release/bundle/CoShell.app (기본은 로컬용 ad-hoc 서명)
scripts/package-dmg.sh     # CoShell-<버전>.dmg
```

배포용은 `COSHELL_SIGN_IDENTITY`를 지정해 번들을 만든 뒤 `scripts/notarize-macos.sh`로 공증합니다(스크립트 상단 주석 참고).

## 단축키 (macOS)

| 키 | 동작 |
|---|---|
| ⌘K | 서버 검색 · 명령 |
| ⌘O | 서버 목록 |
| ⌘T / ⌘W | 새 로컬 셸 / 탭 닫기 |
| ⌘1–9, ⌘⇧[ / ⌘⇧] | 탭 선택, 이전 / 다음 탭 |
| ⌘⇧← / ⌘⇧→ | 탭 순서 변경 |
| ⌘L | AI 패널 |
| ⌘F, ⌘G / ⌘⇧G | 출력 검색, 이전 / 다음 결과 |
| ⌘C / ⌘V | 복사 / 붙여넣기 |
| ⌘+ / ⌘− / ⌘0 | 글꼴 크기 |
| ⌘, | 설정 |
| ⌘ 클릭 | 링크 열기 |

활성 탭을 한 번 더 클릭하면 이름을 바꿀 수 있습니다.

## 데이터 위치

| 내용 | 위치 |
|---|---|
| 서버 목록 | `~/.config/coshell/hosts.toml` |
| 설정 · 창 상태 | `~/.config/coshell/settings.toml` |
| AI 프로필 | `~/.config/coshell/ai.toml` |
| 서버 비밀번호 · API 키 | OS 키체인 (설정 파일에는 저장하지 않음) |

`~/.ssh/config`는 읽기만 하고 수정하지 않습니다. `known_hosts`에는 호스트 키를 신뢰하고 저장할 때만 씁니다.

## 구조

| 크레이트 | 역할 |
|---|---|
| `coshell-term` | 터미널 에뮬레이터 코어 (alacritty_terminal), 로컬 PTY |
| `coshell-ssh` | SSH 접속, 경유 체인, 호스트 키 검증, 포트 포워딩 |
| `coshell-ai` | AI 제공자 클라이언트, 프로필, 비밀 값 마스킹 |
| `coshell-app` | iced UI 애플리케이션 |

## 라이선스

[Apache-2.0](LICENSE)
