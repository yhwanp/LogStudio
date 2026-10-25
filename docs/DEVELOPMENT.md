# LogStudio 개발 문서

설치·실행과 기본 빌드 명령은 [README.md](../README.md), 사용자 기능 규칙은 [USER_GUIDE.md](USER_GUIDE.md), 성능 측정 결과와 미검증 범위는 [LARGE_LOG_VALIDATION.md](LARGE_LOG_VALIDATION.md)를 참고하세요.

## 구성

| 경로 | 역할 |
| --- | --- |
| `core/src/source.rs` | 승인된 파일·붙여넣기 원문 읽기, 여러 파일을 순서대로 이은 단일 바이트 공간(`Part`), 파일 첫 4바이트 기반 형식 판별(`Format`: 텍스트·DLT, 혼합 열기 거부), 앞 64 KiB 기반 iOS 로그 판별(`Part::ios`, `parser::ios_log`), 외부 파일 변경 감지 및 단일 텍스트 파일의 끝부분 추가 확인(`grow`), 공개 오류 메시지 |
| `core/src/order.rs` | 다중 선택·드롭한 파일의 정렬: 파일 끝 64 KiB의 마지막 로그 시각(없으면 앞 64 KiB의 첫 로그 시각, DLT는 첫 메시지 시각) 기준, 시각이 없거나 모호하면 회전 번호를 고려한 자연 이름순 정렬(`order`, `arrange`) |
| `core/src/dataset.rs` | 희소 물리 행 인덱스(약 128 KiB 블록 단위, 파일 경계를 넘지 않음), DLT 단위 인덱스(`index_dlt_file`) 및 블록을 행 텍스트로 변환하는 `read_rows`, pane별 쿼리 실행, 레벨 집계, `page`·`locate`·`range`·`record`·`lines`, 내보내기(`export_as`), 실시간 따라가기 |
| `core/src/fields_index.rs` | 분석 단계에서 생성하는 블록별 PID·TID·태그 메타데이터 표, 필드 필터 적용 시 블록 건너뛰기 및 블록을 읽지 않는 카운트 계산 |
| `core/src/parser.rs` | Android(`threadtime`·`-v time`·`brief`·`-v long`, `-v uid`·`-v year`·`-v usec`·`-v nsec`·`-v zone`·`-v epoch`·`-v monotonic`·`-v color`, Android Studio Logcat 복사본) 및 iOS(`idevicesyslog` 포함) 필드 파싱, 앞쪽 줄 번호 인식, 데이터셋 형식별 파서 선택(`parse_in`·`starts_entry_in`), DLT 행 필드 분리 |
| `core/src/dlt.rs` | DLT(프로토콜 버전 1) 단위 분할(`units`), 헤더 파싱, `dlt-viewer` 호환 페이로드 포매팅 및 행 렌더링(`Decoder::render`), 행 필드 추출(`parse_row`), 로컬 시각 변환(`Clock`), 테스트·벤치마크용 합성 생성기(`synth`) |
| `core/src/search.rs` | 고정 버전 `grep-regex` / `grep-searcher` 기반 문자열·정규식 검색 및 하이라이트 범위 계산, 필터 조건(레벨·태그·PID·TID·시간·메시지) 컴파일과 블록·행 판정 |
| `core/src/encoding.rs` | 표시 텍스트 디코딩: 손상된 UTF-8 바이트의 표시용 대체(``)와 손상 위치 마스크, Mac OS Roman으로 잘못 읽혀 깨진 UTF-8 문자 복원(`repair`), UTF-16·압축 파일 판별 |
| `core/src/launch.rs` | CLI 경로 인자(복수 지정 시 병합 열기) 및 `--large-log`(반복 가능) 옵션 해석 |
| `core/src/capture/` | 기기 기록: `mod.rs`(기록기·재연결 상태 흐름·저장·버리기), `gate.rs`(`threadtime` 시각과 시작 경계·중복 제거), `link.rs`(연결 계층 trait·기기 종류·`adb devices -l` 파서·공개 오류 문구), `cli.rs`(adb 명령 실행 연결), `adb.rs`(adb 탐지·`adb version` 검증), `process.rs`(adb·Xcode 도구 명령을 표준 입력과 Windows 콘솔 창 없이 만들고, 시간 제한 안에 끝까지 실행), `ios.rs`(Apple 기기 서비스(usbmuxd)로 iPhone·iPad 목록과 `os_trace_relay` 로그를 받아 iOS 로그 줄로 쓰는 연결), `apps.rs`(기록 중인 iPhone·iPad의 개발용 서명 앱 목록과 Xcode `devicectl`로 `OS_ACTIVITY_DT_MODE=1` 실행), `store.rs`(임시 기록 파일과 잠금·남은 기록), `fake.rs`(테스트·QA용 가짜 연결과 합성 로그) |
| `core/src/lib.rs` | `Engine`: 파일 열기·검증·롤백 복원, 기기 기록 로그 열기(`open_capture`)와 수명 관리, IPC 요청 처리, 진행 상태 이벤트 발행 |
| `src-tauri/src/main.rs` | Tauri 커맨드 핸들러, 동시 요청 수 제한, 파일 선택·드롭 처리(다중 파일은 `order::arrange` 정렬), 시작 인자 오류 보관(`take_startup_error`) |
| `src-tauri/src/clipboard.rs` | macOS Finder 파일 및 UTF-8 BOM 붙여넣기용 일회성 권한(grant) 발급·연장·폐기 |
| `src-tauri/src/drag.rs` | 네이티브 드래그 이벤트 판정(`decide`): 실제 파일 경로가 있을 때만 드롭 안내 표시 및 열기 수행 |
| `src-tauri/src/geometry.rs` | 창 크기·위치·최대화 상태 저장(`window-state.json`) 및 현재 연결된 모니터 작업 영역 안으로 보정 복원 |
| `src-tauri/src/work.rs` | 파일별 작업 상태 저장(`work-states.json`): 정규 경로·크기·수정 시각으로 식별, 최근 50개 보관, 크기 제한 적용 |
| `src-tauri/src/filters.rs` | 저장된 필터 관리(`saved-filters.json`): 이름 정규화 및 중복 검사, 크기 제한, 원자적 파일 교체 저장 |
| `src-tauri/src/devices.rs` | 기기 기록의 데스크톱 상태: adb 탐지 결과와 사용자가 지정한 adb(`adb.json`), Android 기기와 iPhone·iPad를 함께 담은 마지막 기기 목록(기록은 이 목록의 serial·UDID로만 시작하며, 기기 종류에 맞는 연결을 고름), 기록 중인 iPhone·iPad의 마지막 앱 목록(앱은 이 목록의 bundle ID로만 실행), 앱 캐시의 임시 기록 폴더(`captures/`)와 남은 기록 ID, QA 빌드의 가짜 기기 |
| `src-tauri/src/recent.rs` | 최근 파일 목록 관리(`recent-files.json`): 프론트엔드에는 ID와 표시 이름만 전달하고 검증된 파일만 재열기 허용 |
| `src-tauri/tauri.conf.json`, `src-tauri/capabilities/` | 앱 메타데이터·버전·식별자, 프론트엔드 경로(`frontendDist`), CSP 설정, Tauri 권한 정의 |
| `src-tauri/icons/` | 앱 아이콘 원본(`icon.svg`, `icon-rounded.svg`) 및 생성된 `icon.icns`·`icon.ico`·`icon.png`([빌드와 배포](#빌드와-배포) 참고) |
| `dist/` | 프론트엔드 소스(번들러 없는 순수 ES 모듈, 실행 파일에 그대로 내장). `index.html`이 `styles.css`와 `app.js`를 로드([화면 모듈](#화면-모듈) 참고) |
| `desktop/` | macOS `.app` 패키징, 배포용 ZIP·체크섬 생성, 아이콘 생성, 로컬 빌드·실행 스크립트, 팁 초기화 스크립트 |
| `core/tests/` | Rust 통합 테스트(형식 인식·필터·검색·DLT·멀티라인·따라가기·요청 퍼징 등) 및 파서 골든 데이터(`parser-golden.json`) |
| `tests/` | Node 프론트엔드 단위 테스트(`*.test.mjs`), 패키징·스크립트 Python 테스트(`test_*.py`), 페이지 골든 데이터(`*-page.json`) |
| `tests/fixtures/covesa/` | COVESA 공개 DLT 테스트 샘플(MPL-2.0, 테스트 전용, [상세 설명](../tests/fixtures/covesa/README.md)) |
| `tests/e2e/` | QA 빌드의 내장 WebDriver로 실제 macOS WKWebView를 검증하고 라이트·다크 화면을 캡처하는 E2E 테스트([tests/e2e/README.md](../tests/e2e/README.md)) |
| `bench/` | 합성 로그 생성기 및 코어 엔진 성능 측정 바이너리([bench/README.md](../bench/README.md)) |
| `docs/` | 사용 설명서(`USER_GUIDE.md`), 개발 문서(`DEVELOPMENT.md`), 대용량 로그 검증 기록(`LARGE_LOG_VALIDATION.md`) |
| `extras/` | 릴리스에 포함되지 않는 확장 기능(`--features extras`): 확장 호스트(`src/lib.rs`), 좌표 궤적(`src/track.rs`), 확장 창의 페이지 파일(`web/`)과 내장 Leaflet([EXTRAS.md](EXTRAS.md) 참고) |
| `artifacts/` | Git에서 제외되는 로컬 빌드(`desktop/`) 및 E2E 테스트(`e2e/`) 출력 폴더 |

### 화면 모듈

| 영역 | 파일 및 역할 |
| --- | --- |
| 조립·렌더링 | `app.js`(진입점 및 모듈 연결), `large-view.js`(화면 구성 및 행 렌더링), `continuous-view.js`(가상 연속 스크롤), `columns.js`(텍스트·DLT 열 프로필 및 셀 렌더링), `entries.js`(멀티라인 이어지는 줄 표시 규칙) |
| 상태·통신 | `controller.js`(pane별 쿼리 수명주기 관리, DOM 비의존), `desktop.js`(Tauri IPC 래퍼), `work-state.js`(다시 열기·작업 복원 및 디바운스 저장), `layout.js`(열 폭·하단 높이 저장), `zoom.js`(화면 배율 제어) |
| 필터 | `filter-model.js`(레벨 목록, 붙여넣기 25 MiB 상한, 조건 모델 및 입력 검증), `conditions.js`(필터 패널의 조건 카드 UI), `saved-filters.js`(저장된 필터 직렬화·비교·메뉴·이름 대화상자) |
| 행 선택·목록 | `row-selection.js`(상단·검색·북마크 공통 행 선택 상태 머신, DOM 비의존), `row-gestures.js`(마우스 클릭·드래그·자동 스크롤), `bookmarks.js`(북마크 관리), `recent.js`(최근 파일 메뉴) |
| 공통·유틸 | `dom.js`(DOM 헬퍼), `demo.js`(빈 화면의 `샘플 로그 보기`용 합성 로그) |
| 기기 기록 | `devices.js`(Android 기기와 iPhone·iPad를 종류별로 보여 주고, USB 디버깅 허용·신뢰·잠금 해제를 기다리는 기기가 있으면 열린 동안 다시 조회하는 기기 메뉴, adb·Apple 기기 서비스 설치 안내, 상단 바 기록 상태와 화면 낭독기용 설명, 기록 중인 iPhone·iPad의 `앱 실행…` 메뉴(macOS만)와 실행 지점 표시, 저장 확인 대화상자, 남은 기록 목록, 드롭·창 닫기 보류 응답) |
| 확장 기능 | `extras.js`(extras 빌드에서 설정 파일이 있는 확장 기능의 `확장` 메뉴와 설정 오류 표시·`설정 폴더 열기`, 창으로 돌아올 때 목록 갱신, 선택한 행을 확장 창에 전달, 확장 창이 고른 행을 상단 로그에서 선택; 다른 빌드에서는 버튼을 숨김) |
| 메뉴·대화상자·문구 | `menu.js`(팝업 메뉴), `shortcuts.js`(단축키 안내 대화상자), `clipboard.js`(붙여넣기 대상 판별: 입력란·로그 텍스트·파일 경로), `paste-confirm.js`(열린 로그 교체 확인 대화상자), `tips.js`(첫 사용 팁 및 기능 둘러보기), `describe.js`(진행 상태·오류·필터 요약 문구) |

## 처리 구조

파일 읽기·파싱·필터링·검색은 Tauri에 링크된 단일 Rust 코어(`core/`)가 수행하며, 프론트엔드(`dist/`)는 화면 렌더링과 사용자 입력만 담당합니다. QA WebDriver와 픽스처 열기 커맨드는 `--features qa` 빌드에 한해 포함됩니다. 확장 기능은 `--features extras` 빌드에만 포함되며, 코어의 `Engine::shown`과 `Dataset::read_blocks`로 화면의 로그를 읽습니다([EXTRAS.md](EXTRAS.md)).

### 인덱싱과 스캔

- **블록 인덱싱:** 원문을 약 128 KiB 크기의 줄 경계 블록으로 나눕니다. 블록마다 UTF-8 유효성, NUL 바이트 여부, 최대 행 길이(1 MiB)를 검사한 뒤 게시하며, 열기를 확정하기 직전에 외부 파일 변경 여부를 한 번 더 확인합니다.
- **표시 텍스트 디코딩:** `encoding::decoded`가 블록과 행의 바이트를 표시용 문자열로 바꿉니다. 손상된 UTF-8 바이트는 U+FFFD로 대체하고, iOS 로그 파일의 행이면 Mac OS Roman으로 잘못 읽혀 깨진 문자도 복원합니다.
  - **iOS 파일 판별:** 파일을 열 때 앞 64 KiB에서 인식된 행 중 iOS 행이 다른 형식 행 이상이면 `Part::ios`를 켭니다(`parser::ios_log`). Android 파일과 DLT는 복원하지 않습니다.
  - **복원 조건:** Mac OS Roman 상위 문자가 이어진 구간에서 올바른 UTF-8을 이루는 가장 긴 앞부분을 복원하되, `plausible` 검사를 통과할 때만 적용합니다. 결과가 두 바이트 문자 하나면 U+00A0–024F여야 하고, 결합 문자로 시작하거나 C1 제어 문자·U+2028/2029가 들어 있으면 복원하지 않습니다.
  - **일관성:** 복원 여부는 `Rows::repair`로 블록 스캔(`Filter::mask_from`·`row_hits`), 레벨·필드 분석, 페이지, 원문 상세에 똑같이 전달되므로 표시와 검색·필터 결과가 일치합니다. ASCII와 줄 구분 문자는 바뀌지 않아 물리 행 경계와 엔트리 시작 판정(`opens_entry`)도 그대로입니다.
  - 바이트 위치(`startByte`·`sourceStartByte`)와 내보내기는 원본 바이트 기준이며, 원문 상세 응답은 복원한 문자가 있으면 `encodingRepaired: true`를 포함합니다.
- **즉시 표시:** `page` 조회와 쿼리 스캔은 전체 파일 인덱싱이 끝나기를 기다리지 않고 이미 게시된 블록부터 즉시 읽습니다. 전체 레벨 통계 및 필드 메타데이터 분석은 인덱싱 완료 후 워커 스레드에서 이어서 실행됩니다.
- **병렬 스캔:** 각 pane마다 전용 쿼리 스레드가 하나씩 동작하며, 블록 판정은 최대 4개(DLT는 최대 8개, CPU 코어 수의 절반 이하) 워커가 병렬로 처리합니다. 일치 결과는 전체 행 번호 배열 대신 블록별 일치 건수(`ends`)와 비트마스크로 관리해 메모리 사용량을 낮추고 원문 순서를 유지합니다.
- **순차 미리 읽기(`blocks_in_order`, `Source::read_ahead`):** 워커는 다음 블록을 하나씩 번갈아 가져가므로 OS가 하나의 순차 읽기 흐름으로 보지 못하고, OS 파일 캐시보다 큰 파일은 매번 디스크에서 느리게 읽힙니다. 그래서 패스(쿼리 스캔·분석)에서 원문을 실제로 읽은 블록이 처음 나오면, 스레드 하나가 패스 전용 핸들로 그 뒤 블록들을 워커보다 최대 64블록(약 8 MiB) 앞서 순서대로 읽어 캐시에 올립니다.
  - 읽은 바이트는 버리고, 워커는 지금처럼 `Source::read`(변경 확인 포함)로 읽습니다. 핸들은 파일마다 경로로 다시 열고 처음 허용한 파일과 같은지 확인하며, Windows에서는 같은 공유 모드에 `FILE_FLAG_SEQUENTIAL_SCAN`을 붙입니다. 패스가 도는 동안 핸들을 하나만 더 쥡니다(파일을 차례로 바꿔 열며, 두 pane과 분석을 합쳐 최대 3개). 원래 핸들과 같은 공유 모드로 여므로 열린 파일의 이름 변경이나 삭제를 막지 않습니다. 붙여넣기 텍스트는 미리 읽지 않습니다.
  - 인덱싱이 파일을 읽는 동안에는 기다립니다(인덱서와 경합하면 인덱싱이 늦어짐). 표로 판정되는 블록은 읽지 않고 넘어갑니다. 워커보다 뒤처져도 워커 위치로 뛰어넘지 않는데, 그러면 OS의 순차 미리 읽기가 끊겨 오히려 느려지기 때문입니다.
  - 워커가 모두 끝나거나 취소·닫기·`hold`가 되면 바로 멈추고, 표만으로 끝나는 패스에서는 띄우지 않습니다.
- **필터 스캔과 분석 합치기(`analyze_block`, `wanted`):** 검색어 없이 필드 조건으로 행을 판정하는 텍스트 쿼리(`needs_fields`)는 표가 판정하지 못한 블록의 행을 모두 파싱하므로, 분석이 끝나기 전이라면 같은 파싱으로 그 블록의 레벨 통계·필드 메타데이터도 만들어 분석 상태에 블록 순서대로 이어 붙입니다. 작업 복원으로 필터가 파일과 함께 시작할 때 같은 행을 두 번 파싱하지 않기 위해서입니다.
  - 스캔은 아무 패스도 쥐고 있지 않은 분석 상태를 가져가 앞서 분석된 블록 다음부터 이어 가고, 분석 패스는 양보하던 중 이런 스캔이 있으면(`wanted`) 지금까지의 상태를 넘기고 멈춥니다.
  - 스캔이 중단 없이 끝나면 분석 패스처럼 요약과 표를 게시하고(따라가는 파일에 그사이 붙은 블록은 다음 패스가 이어 감), 취소되면 상태를 돌려준 뒤 분석 패스가 나머지 블록을 이어 갑니다(마지막 블록까지 마친 뒤 취소됐으면 게시만 합니다). 따라가는 파일에서 이어 가는 스캔처럼, 스캔의 첫 블록 앞부분을 상태가 아직 분석하지 않았으면 합치지 않습니다. 그 블록들은 이 스캔이 다시 읽지 않기 때문입니다.
  - 행 판정은 `Filter::mask_from`과 같은 규칙(번호 인식 시 번호 접두사 먼저 파싱, 이어지는 줄은 소속 로그의 판정)을 따르며, `field_index` 테스트가 모든 필터에서 두 경로의 결과·요약·표가 같은지 비교합니다. DLT는 헤더만으로 판정해 행을 렌더링하지 않으므로 합치지 않습니다.
- **제로카피 필드 판정:** 필터 판정 시에는 원문 슬라이스를 빌린(`borrow`) 필드를 사용하며, JSON 행 객체는 화면에 실제로 보낼 페이지 행에 대해서만 생성합니다.
- **파서 우선순위:** 각 행에 대해 `threadtime`과 iOS(연도로 시작하면 iOS 우선) 파서를 먼저 시도하고, 둘 다 실패한 행에만 `parser::later`의 나머지 형식을 시도합니다. 시각으로 시작하는 줄(`stamped`)은 `-v time`·`-v uid`·Android Studio 형식, 그리고 시간대·epoch·monotonic 시각이 붙은 `threadtime`·`-v time`입니다. 그 밖의 형식은 `-v long` 머리 줄(`long`), `brief`, `idevicesyslog`(`syslog`)입니다. 줄 앞 몇 바이트만으로 대상이 아닌 줄을 빠르게 걸러내 정규식 오버헤드를 피하므로, 새 형식을 더해도 기존 형식 행의 처리 경로는 바뀌지 않습니다.
  - **색상 코드:** `-v color`가 줄 앞뒤에 붙이는 SGR 코드는 필드를 읽기 전에 건너뜁니다(`uncolored`). 필드는 여전히 원문 슬라이스를 가리키므로 원문 바이트는 바뀌지 않고 `fieldOffsets`도 원문 위치로 계산됩니다. 줄 앞에 코드가 없으면 메시지 안의 코드도 건드리지 않습니다.
  - **`idevicesyslog` 행:** iOS 행(`Parsed::ios`)이지만 날짜에 연도가 없어 행 JSON의 `year`가 `null`입니다. 화면은 `year` 키가 있는지로 iOS 행 배치(프로세스 칸 폭, 태그 끝부분 보이기)를 고르므로 키를 빼지 않습니다.
  - **정규식 상태 재사용:** 파서 정규식은 `regex-automata`의 meta 엔진을 직접 쓰고, 정규식마다 스레드별 검색 캐시와 캡처 슬롯을 하나씩 두어 재사용합니다(`parser::read`). `regex::Regex::captures`는 호출마다 캡처 객체를 할당하고 워커들이 함께 쓰는 캐시 풀을 거치므로, 모든 행을 파싱하는 집계·필드 필터 스캔에서는 이 비용이 매칭 자체만큼 들었습니다([측정](LARGE_LOG_VALIDATION.md#파서-정규식-상태-재사용)). 정규식 문법과 매칭 결과는 같습니다.

### 필터 조건과 메타데이터 가속

- **포함·제외 조건 모델:** `Filters`는 포함 조건(`include`)과 제외 조건(`exclude`) 목록(각 `Condition`은 `levels`·`tag`·`pid`·`tid`·`timeFrom`·`timeTo`·`message`·`messageRegex` 보유, 합산 최대 32개)으로 구성됩니다. 행은 포함 조건 중 하나에 일치하고(포함 조건이 비어 있으면 모든 행 통과) 제외 조건에는 하나도 일치하지 않아야 남습니다. 하위 호환용 단일 필드(`levels`~`tid`, `excludeTags`)는 `Filter::new`에서 동일한 조건 목록 형태로 변환해 컴파일합니다.
- **다중 값 매칭:** 조건의 `tag`(쉼표 구분)와 `pid`·`tid`(쉼표·공백 구분)는 앞뒤 공백·빈 값·중복을 제거해 최대 32개까지 파싱합니다. 태그 목록은 대소문자를 무시하는 리터럴 alternation 정규식 하나로 묶어 매칭하고, PID·TID는 단일 값이면 문자열 비교, 복수 값이면 정렬된 배열의 이진 탐색으로 판정합니다.
- **메시지 조건:** `message`는 파싱한 행의 메시지 필드(DLT는 페이로드)에 들어 있어야 할 글자로, 검색과 같은 `grep-regex` 매처를 대소문자 무시로 만듭니다(`messageRegex`면 정규식, 아니면 고정 문자열). 입력 문자열은 앞뒤 공백을 제거하지 않고 그대로 쓰며(화면 최대 1,000자, 엔진 상한 4,000바이트), 메시지가 비어 있으면 `messageRegex`를 끈 정규형으로 비교합니다. 정규식 오류는 `Invalid or unsupported message pattern: …`으로 돌려주며, 엔진은 필터를 컴파일한 뒤에야 pane의 이전 쿼리를 교체하므로 화면은 거부된 필터 대신 이전 쿼리와 위치로 돌아갑니다(`controller.js`의 `rejected`).
  - 메시지는 표로 판정할 수 없어 시간 범위처럼 `Cond::reads_rows`가 참이고, 표는 다른 필드로 통과할 수 없는 블록만 건너뜁니다.
  - **고정 문자열 선별(`Filter::message_rows`·`mask_by_messages`):** 검색어가 없고, 모든 보기 조건에 고정 문자열 메시지가 있거나(보기 조건이 없고 `미인식 줄 포함`이 켜져 있으면) 모든 제외 조건에 고정 문자열 메시지가 있으면, 블록에서 그 글자들을 먼저 grep으로 찾습니다. 해당 문자열이 없는 엔트리 시작 행은 파싱하지 않고 기본값으로 정합니다(보기 조건이면 숨김, 제외 조건이면 통과). 이어지는 줄은 소속 엔트리의 판정을 따르므로, 파싱하는 행은 문자열이 발견된 행, 블록의 첫 엔트리 시작 행 앞의 줄, 기본값과 다르게 판정된 엔트리에 이어지는 줄뿐입니다. 정규식은 `^`·`\b` 같은 앵커가 행 전체와 메시지에서 다르게 맞을 수 있어 이 경로를 쓰지 않습니다. `row_hits`도 블록에 글자가 없는 조건은 그 블록에서 빼고, 남은 조건이 모두 고정 문자열이면 글자가 있는 행만 읽습니다.
  - DLT 블록은 메시지 조건이 있으면 헤더만 읽는 `dlt_mask`·`dlt_row_hits` 대신 블록을 렌더링해 텍스트 행과 같은 판정을 씁니다.
- **블록별 필드 메타데이터(`fields_index.rs`):**
  - 레벨 통계 분석 패스에서 블록별·줄 번호 인식 옵션별로 `(필드 값, 레벨)` 행 수를 집계해 사전과 함께 게시합니다(이어지는 줄은 소속 로그의 필드 값을 따르고 고아 줄은 제외).
  - 쿼리 시작 시 각 조건의 판정 함수(`accepts_tag`·`accepts_pid`·`accepts_tid`)를 사전의 고유 값들에 대해 한 번씩만 평가해 비트셋을 만듭니다.
  - 표는 처음 모든 블록을 마친 분석 패스가 게시하므로, 따라가는 파일의 뒤에 붙은 블록에는 사전에 없는 값이 있을 수 있습니다. 그래서 조건이 검사하는 필드의 표가 다루지 않는 블록은 표로 판정하지 않고 읽습니다(`FieldIndex::covers`). 사전의 모든 값이 조건을 통과하더라도(빈 사전이면 늘 그렇습니다) 그 블록의 행까지 통과한다고는 할 수 없기 때문입니다. 비어 있는 파일(기기 기록의 시작)을 분석한 패스는 빈 표를 게시하고, 첫 행들을 분석하는 패스가 값을 모아 다시 게시합니다.
  - 블록마다 조건별로 통과 가능한 `(값, 레벨)` 쌍이 하나도 없으면 해당 블록은 읽지 않고 건너뜁니다. 시간 범위나 검색어가 없고 활성 조건들이 레벨과 단일 필드만 참조하는 경우에는 블록 원문을 읽지 않고 메타데이터 표만으로 즉시 건수를 계산합니다.
  - 필드 하나의 고유 값 사전이 65,536개를 넘거나 메타데이터 크기가 약 32 MiB를 초과하면 해당 필드 표만 폐기하고 블록 스캔 방식으로 폴백합니다.
- **일치하는 줄이 없는 조건(0건 조건) 확인:**
  - 상단(`logs`) pane 쿼리는 활성 `include`·`exclude` 조건 각각에 대해 현재 데이터셋 전체에서 일치하는 줄이 하나라도 있는지 별도로 확인합니다(`some` / `none` / `pending`).
  - 레벨 표와 필드 표(`FieldIndex::probe`)만으로 판정 가능한 조건은 즉시 확정하고, 시간 범위·메시지나 복수 필드 조건처럼 블록을 직접 읽어야 하는 미확정 조건들은 남은 블록을 한 번에 함께 스캔합니다(`Filter::row_hits`, DLT는 메시지 조건이 없으면 헤더만 `dlt_row_hits`).
  - 확인 작업은 진행 중인 쿼리 스캔에 양보하며, 한 번 확인된 결과는 `(줄 번호 인식 옵션, 조건 정규형)` 단위로 데이터셋에 캐시되어 다른 조건을 수정해도 재계산 없이 바로 재사용됩니다.

### 열기·교체 및 다중 파일 정렬

- **취소와 세대 관리:** 스캔 워커는 블록 전후와 매치 콜백마다 취소 플래그를 확인합니다. 새 쿼리는 이전 쿼리 스레드가 종료(join)된 뒤 시작하며, `datasetId`·`queryId`가 일치하지 않는 이전 응답은 코어와 화면 양쪽에서 폐기합니다.
- **열기 실패 시 롤백:** 새 파일을 여는 동안 직전에 검증 완료된 소스를 일시 중지 상태로 보관했다가, 새 파일 열기가 실패하거나 취소되면 이전 소스를 즉시 복원합니다(동시 보유 파일 핸들은 최대 2세트).
- **다중 파일 정렬(`order::arrange`):** 파일 선택 창이나 드래그 앤 드롭으로 받은 2~64개 파일은 엔진에 전달하기 전에 파일당 앞·뒤 64 KiB만 읽어 시각순(또는 회전 로그 번호를 반영한 자연 이름순)으로 정렬합니다. 부팅 직후 시계가 맞춰지기 전의 초기 로그가 파일 앞부분에 섞여 있어도 순서가 깨지지 않도록 파일 끝부분의 마지막 로그 시각을 우선 사용합니다(DLT는 첫 메시지 시각 사용).

### 외부 파일 변경 감지와 실시간 따라가기

- **파일 변경 감지:** 블록을 읽을 때마다 파일 크기와 메타데이터를 대조합니다. 단일 텍스트 파일은 파일 끝에 바이트가 추가된 경우만 허용하며, 파일 식별자(Unix `dev`/`ino`, Windows 생성 시각 및 경로)와 기존 구간의 처음·끝 4 KiB가 그대로인지 확인합니다. 다중 병합 파일은 어떠한 변경도 허용하지 않습니다.
- **Windows 파일 공유:** Windows에서는 읽기·쓰기·삭제 공유 모드로 파일을 열어, 다른 프로그램이 기록 중인 로그를 열거나 열려 있는 동안 파일 이름이 바뀌거나(로그 회전) 삭제되는 것을 막지 않습니다.
- **실시간 따라가기(`follow`):** 단일 텍스트 파일에서 따라가기를 켜면 전용 폴링 스레드가 500 ms마다 `Source::grow`로 파일 증가분을 확인하고 새 바이트만 이어서 인덱싱합니다. 아직 줄바꿈이 없는 마지막 미완성 줄은 보류하며, 처음 열 때 줄바꿈 없이 끝났던 마지막 행에 새 바이트가 이어져 완성되면 해당 마지막 블록만 되돌린 뒤 다시 게시합니다(`tailRevision` 증가). 쿼리 결과와 레벨·필드 분석 상태는 마지막 블록 이후의 carry 상태를 보관해 처음부터 다시 스캔하지 않고 증분 처리합니다.

### 기기 기록

연결한 Android 기기의 logcat이나 iPhone·iPad의 로그를 앱 캐시의 임시 파일에 기록하고, 그 파일을 기존 [실시간 따라가기](#외부-파일-변경-감지와-실시간-따라가기)로 엽니다. 인덱싱·쿼리·집계는 일반 파일과 같은 경로를 쓰고, 기기 읽기는 인덱싱·화면과 분리된 스레드에서 이루어집니다.

- **연결 계층(`link::Link`):** 기기 목록, `boot_id`, 기기 고유 번호(`ro.serialno`), 최근 로그(`logcat -d -t N`), 스트림(`logcat -v threadtime [-T 시각]`)만 요청합니다. 연결은 스트림이 열리기 전의 줄을 보내는지(`replays`)를 알리며, 보내지 않는 연결(iPhone·iPad)은 시작 경계 없이 받은 줄을 모두 씁니다. Android는 adb 명령 실행(`cli::AdbCli`)을 쓰며, 서버는 출력을 쓰지 않는 `adb start-server`로 먼저 띄워 서버가 앱의 파이프를 물려받지 않게 합니다. 스트림을 뺀 명령은 시간 제한 안에 끝나야 하고, 모든 명령은 Windows에서 콘솔 창 없이 실행하며 앱 환경의 `ANDROID_LOG_TAGS`는 지웁니다.
- **시작 경계(`gate.rs`):** 기기 버퍼의 최근 줄 가운데 마지막 시각을 경계로 `-T`를 요청하고(`-T`는 그 시각의 줄을 포함), 경계 시각의 줄 중 이미 있던 줄은 원문 횟수만큼 버립니다. 1 MiB를 넘게 버리면 그 뒤로는 모두 통과시켜 줄을 잃지 않습니다.
- **기록기:** 받은 바이트를 변환하지 않고 LF로 끝난 줄만 파일 끝에 추가합니다. 연결이 줄 중간에서 끊기면 그 조각은 다음 연결이 다시 보내는 온전한 줄로 대체되고, 기록을 멈출 때는 받은 바이트를 모두 씁니다. 8 GiB·디스크 부족·쓰기 실패는 기록을 멈춥니다.
- **재연결:** 스트림이 끝나면 기기를 1초마다 찾습니다. 같은 `boot_id`면 기록 파일 끝의 마지막 시각을 경계로 이어 붙이고, 다르면(재부팅) 새 부팅의 처음부터 받습니다. adb의 종료 코드와 오류 출력만으로는 끊긴 원인을 구분할 수 없으므로(에뮬레이터에서 adb 서버 종료와 재부팅 모두 종료 코드 255, 오류 출력 없음) 다시 연결해 보면서 판단합니다. 기다리는 동안 같은 serial의 기기가 미승인·오프라인으로 보이면 그 이유를 상태에 담아 화면에 표시합니다. adb 버전 불일치 문구를 보면 재연결을 멈춥니다. 끊긴 지점(다음 첫 행 번호·시각·재부팅 여부)은 상태로만 보관합니다.
- **iPhone·iPad(`ios::IosLink`):** Apple 기기 서비스(usbmuxd: macOS는 `/var/run/usbmuxd`, Windows는 Apple Devices 앱이나 iTunes가 설치하는 Apple Mobile Device Service의 `127.0.0.1:27015`)에 [`idevice`](https://github.com/jkcoxson/idevice) 크레이트(MIT)로 연결합니다. 0.2 전까지는 릴리스마다 API가 바뀌므로 정확한 버전으로 고정합니다.
  - **목록과 신뢰 확인:** USB나 Wi-Fi로 연결한 기기를 UDID당 하나씩 담고, USB와 Wi-Fi로 함께 보이면 USB 연결을 씁니다. 연결 종류를 알 수 없는 기기는 뺍니다. Wi-Fi 기기도 usbmuxd가 같은 방식으로 중계하므로 연결하는 과정은 USB와 같습니다. 기기 정보는 기기마다 3초 안에 동시에 읽고, 그 안에 답하지 않는 기기(네트워크를 벗어난 Wi-Fi 기기 등)는 `offline`으로 담아 요청 전체의 5초 제한에 걸려 다른 기기까지 목록에서 사라지지 않게 합니다. 세션 전 lockdown 값으로 이름·제품·iOS 버전을 읽고, 페어링 기록(`ReadPairRecord`)과 세션 시작으로 `device`·`untrusted`·`locked`를 판단합니다.
  - **로그 스트림:** `com.apple.os_trace_relay`에 `StartActivity`(모든 프로세스·모든 레벨)를 요청하고, 받은 이진 레코드를 직접 경계 검사하며 읽어 `날짜 시각.마이크로초 프로세스[PID:TID] [레벨] [subsystem:category] 메시지` 줄로 씁니다. 시각은 이 컴퓨터의 시간대로 바꾸고, 기본 레벨에는 레벨 칸을 쓰지 않으며, 메시지 바이트는 그대로 둡니다. 라이브러리의 레코드 파서는 손실 변환(`from_utf8_lossy`)과 경계 검사 없는 자르기를 하므로 쓰지 않습니다.
  - **런타임:** 비동기 라이브러리는 요청마다 단일 스레드 tokio 런타임에서 시간 제한을 두고 돌리고, 스트림은 읽기 중에만 런타임을 돌리며 닫기는 `Notify`로 깨웁니다. 라이브러리 패닉은 오류로 바꿉니다.
  - **개발용 서명 앱 실행(`apps.rs`):** 기록 중인 기기에서 고른 개발용 서명 앱을 `OS_ACTIVITY_DT_MODE=1`로 다시 실행해, 그 프로세스의 로그가 `<private>` 없이 `os_trace_relay`로 오게 합니다. 다른 프로세스는 계속 가려지고, 이미 기록한 줄은 바뀌지 않습니다. Apple은 이 변수를 내부 구현이라 하므로 iOS 업데이트로 동작이 바뀔 수 있습니다.
    - **목록:** usbmuxd로 `com.apple.mobile.installation_proxy`에 `Browse`(`ApplicationType` `User`)를 요청해, 서명 권한(`Entitlements`)에 `get-task-allow`가 `true`인 앱만 담습니다. 이 조회는 USB와 Wi-Fi에서 같고 Xcode를 쓰지 않지만, 실행할 수 없는 목록을 보여 주지 않도록 앱 목록을 열 때 Xcode를 먼저 확인합니다. `devicectl device info apps`의 `builtByDeveloper`는 App Store 앱에도 `true`여서 쓰지 않습니다([측정](LARGE_LOG_VALIDATION.md#개발용-서명-앱-실행-os_activity_dt_mode)).
    - **실행 지점:** 데스크톱은 실행을 시작할 때의 다음 행 번호와 시각을 잡아 두었다가, 실행이 성공하면 기록에 앱 이름·프로세스 이름(실행 파일 이름)과 함께 남깁니다(`Capture::mark_launch`, 기록 상태의 `launches`). 새 프로세스의 줄은 이 행 뒤에만 옵니다. 화면은 재연결 구분선과 같은 방식으로 그 행 위에 표시하며, 기록 파일에는 아무것도 쓰지 않습니다.
    - **실행:** Xcode의 `devicectl device process launch --device <UDID> --terminate-existing --environment-variables {"OS_ACTIVITY_DT_MODE":"1"} --json-output <임시 파일> --quiet -- <bundle ID>`를 셸 없이 실행하고, JSON 결과(`info.outcome`, `error.userInfo`의 설명·실패 이유)로 성공과 이유를 판단합니다. `CoreDeviceError` 1000(기기를 찾지 못함)은 Xcode가 그 기기에 연결하지 못한다는 안내로 바꿉니다. 잠긴 기기 같은 실패 이유는 최상위 오류에 없으므로, 중첩된 `NSUnderlyingError`를 따라가 잠김 여부(`FBSOpenApplicationErrorDomain` 7, `Locked`)와 가장 안쪽 실패 이유를 찾습니다.
    - **Xcode 찾기:** `DEVELOPER_DIR`, `/usr/bin/xcode-select --print-path`, `/Applications/Xcode.app/Contents/Developer` 순서로 `usr/bin/devicectl`이 있는 폴더를 씁니다. `xcrun`은 개발자 도구가 없으면 설치를 제안하므로 실행하지 않습니다. Xcode의 `devicectl`은 CoreDevice 버전이 맞지 않으면 `xcodebuild -runFirstLaunch`(시스템 구성 요소 설치)를 먼저 실행하는 셸 스크립트이므로, 실행 전에 `xcodebuild -checkFirstLaunchStatus`로 첫 실행 설정이 끝났는지 확인하고, 끝나지 않았으면 Xcode를 열라고 안내합니다.
    - **`idevice`로 직접 실행하지 않는 이유:** iOS 17부터 앱 실행(DVT `processcontrol`)은 RSD의 `com.apple.instruments.dtservicehub`에만 있고, 이 서비스는 CoreDevice 터널(USB는 lockdown의 CoreDeviceProxy와 사용자 공간 TCP, Wi-Fi는 Xcode가 따로 맺는 RemotePairing TLS-PSK 페어링)과 기기별로 Apple TSS 서명을 받아 마운트하는 개발자 디스크 이미지(Xcode가 `/Library/Developer/DeveloperDiskImages`에 설치)가 있어야 열립니다. Xcode 없이 하려면 디스크 이미지를 내려받고 페어링을 새로 만들어야 해서 사용자가 모르는 도구 설치와 기존 페어링 변경을 피하는 원칙에 어긋나고, Xcode가 있으면 `devicectl`이 이 과정을 모두 처리합니다. 개발용 서명 앱은 대개 Xcode로 빌드하므로 Xcode를 요구해도 실제 사용에는 거의 제약이 없습니다. 다만 이 기능은 macOS에서만 동작합니다.
  - **재연결과 레벨 이름:** 기기는 연결된 동안의 로그만 보내므로 재연결 뒤에는 그때부터 이어서 기록하고, 끊긴 지점에 `gap`을 표시합니다. 재부팅은 구분하지 않습니다. Wi-Fi 기기도 연결이 끊기면 usbmuxd가 연결을 닫아 스트림이 끝나므로(macOS에서 iPhone의 Wi-Fi를 꺼서 확인), 데이터가 오지 않는 시간으로 끊김을 판단하지 않습니다. 데이터셋 상태의 `ios`(모든 파일이 iOS 로그)나 기록의 `platform`이 iOS면 화면은 F·U 레벨을 `Fault`·`Default`로 부릅니다.
- **엔진 연동:** `Engine::open_capture`가 기록 파일을 데이터셋으로 열고 검증 직후 따라가기를 켭니다. 기록이 끝나면 따라가기를 꺼 마지막 줄까지 읽어 건수를 확정하며, 데이터셋을 닫거나 교체가 확정되면 기록을 닫습니다. 롤백으로 되살아난 기록 로그는 일시중지 직전에 따라가고 있었을 때만 다시 따라갑니다.
- **임시 파일(`store.rs`):** 기록 파일 옆 잠금 파일(`.lock`)을 실행 중인 앱이 잡고 있으며, 잡히지 않은 파일은 비정상 종료로 남은 기록입니다. 닫을 때 사용자가 버리기로 했거나 저장하지 않은 바이트가 없으면 파일을 지웁니다.
- **닫기와 종료:** 기록 로그가 열려 있으면 드롭과 창 닫기를 보류하고 화면에 묻습니다(`drop-held`·`close-held`). macOS의 기본 종료 메뉴 항목은 창 닫기를 거치지 않고 앱을 바로 끝냅니다. 그래서 앱 메뉴의 종료(`Cmd+Q`)는 창을 먼저 닫는 메뉴 항목으로 바꿔, 창을 닫을 때와 같은 저장 확인을 거치게 합니다. Dock의 종료와 로그아웃은 여전히 바로 끝나며, 그 기록은 다음 실행의 남은 기록이 됩니다.

### DLT 파일 처리

파일 첫 4바이트가 `DLT\x01`(또는 `DLT\x02`)이면 DLT 소스로 처리합니다.

- **단위 분할과 블록 인덱싱:** `dlt::units`가 바이트 스트림을 메시지(스토리지 헤더 + `16 + LEN`), 비정상 바이트 구간(최대 64 KiB 단위), 버전 2 데이터 단위로 분할합니다. 모든 블록은 항상 단위 경계에서 끝나므로 블록 바이트만 독립적으로 다시 읽어도 동일한 단위와 행이 보장됩니다. 텍스트로 분류된 파일이 NUL 바이트로 실패할 때는 `dlt::headerless`로 스토리지 헤더가 없는 DLT 파일인지 추가로 확인해 정확한 안내 문구를 반환합니다.
- **온디맨드 행 렌더링:** 텍스트 파일은 원본 바이트를 그대로 읽고, DLT 파일은 블록을 읽는 시점(`Dataset::read_rows`)에 `Decoder::render`로 각 단위를 LF로 끝나는 UTF-8 행 문자열(`날짜 시간 타임스탬프 카운터 ECU APID CTID 세션 종류 [페이로드]`)로 변환합니다. 로컬 시간대 오프셋은 데이터셋 안에서 UTC 1시간 단위로 캐시해 렌더링의 결정성(determinism)을 보장합니다.
- **헤더 판정 최적화:** 검색어가 없는 필터 스캔에서 메타데이터 표로 판정할 수 없는 블록(시간 필터, 32 MiB 상한 초과로 태그 표가 폐기된 대용량 DLT 등)은 페이로드를 문자열로 렌더링하지 않고 `Filter::dlt_mask`가 메시지 헤더만 읽어 빠르게 판정합니다. 페이로드 조건(`message`)이 있으면 블록을 렌더링해 판정합니다.

## 화면과 엔진 사이의 요청 (IPC)

프론트엔드는 `log_request` 커맨드 하나로 엔진에 작업을 요청합니다. 모든 요청에는 `action`, `pane`(`logs` 상단 필터, `search` 하단 검색), `datasetId`, `queryId`가 포함됩니다.

| `action` | 설명 |
| --- | --- |
| `status` | 인덱싱 진행률, 레벨 통계, 현재 쿼리 상태 조회(열린 파일이 없으면 `{empty:true}`) |
| `query` | 필터·검색 쿼리 시작(이전 쿼리는 취소·교체). `filters.recognizeLineNumbers`는 `null`/생략(자동), `true`(켜기), `false`(끄기). 필터가 잘못되면(메시지 정규식 오류 등) 이전 쿼리를 그대로 두고 오류만 돌려줌 |
| `page` | 결과 순번 기준 최대 200행 조회. 상단 `logs` pane은 하단 검색의 `searchQueryId`를 함께 전달해 일치 행 강조(`highlights`, `searchHit:true`)를 함께 받을 수 있음 |
| `locate` | 물리 행 번호의 현재 필터·검색 결과 내 순번 조회(결과에 없으면 `found:false`와 다음 결과 위치 `index` 반환) |
| `range` | 단일 물리 행의 원문 조회(최대 64 KiB 구간) |
| `record` | 선택한 물리 행이 속한 멀티라인 로그 엔트리 전체(최대 1 MiB·10,000줄)의 64 KiB 구간 조회(`first`·`last`·`recordComplete` 포함) |
| `lines` | 지정한 오름차순 물리 행 번호 목록(`lineNumbers`, 최대 200개)의 행 데이터를 필터와 무관하게 조회(복원된 북마크 행 로드용) |
| `follow` | `follow:true`/`false`로 실시간 따라가기 켜기·끄기(단일 텍스트 파일 전용) |
| `cancel`, `close` | 진행 중인 쿼리 취소 및 파일 닫기(동시 요청 수 제한과 별도로 즉시 처리) |

기기 기록은 별도 Tauri 명령을 씁니다: `adb_status`·`pick_adb`(adb 탐지·지정), `list_devices`(Android 기기와 iPhone·iPad 목록, 종류별로 목록을 받지 못한 이유), `start_capture`·`stop_capture`, `save_capture`·`discard_capture`(저장·버리기 표시), `capture_leftovers`·`open_leftover`·`delete_leftover`(남은 기록, ID로만 지칭), `capture_apps`·`launch_capture_app`(기록 중인 iPhone·iPad의 개발용 서명 앱 목록과 실행, 데이터셋 ID와 마지막 앱 목록의 bundle ID로만 지칭), `finish_drop`·`answer_close`(기록 로그가 열려 있을 때 보류한 드롭과 창 닫기에 대한 화면의 답). 엔진은 기록 상태가 바뀌면 `capture-changed` 이벤트를 보내고, 상태 응답과 `query-progress`에는 `capture`가 함께 실립니다.

동시 데이터 요청은 최대 4개까지 허용하며, 초과 시 `Local engine is busy; retry`를 반환해 프론트엔드가 짧게 재시도합니다. 엔진은 상태 변화 시 `source-changed`(`opened`·`validated`·`rollback`·`error`), `files-ordered`, `query-progress` 이벤트를 발행합니다.

진행 완료 플래그 세 가지는 각각 독립적인 단계를 나타냅니다.

- `textComplete`: 파일 끝까지 UTF-8 및 물리 행 경계 인덱싱 완료
- `query.complete`: 현재 필터·검색 쿼리가 파일 끝까지 스캔 완료
- `summaryComplete`: 전체 레벨 집계 및 필드 메타데이터 분석 완료

## 연속 스크롤과 화면 동작

- **가상 스크롤(`continuous-view.js`):** 각 목록은 현재 뷰포트 주변의 최대 128개 DOM 행 노드만 유지하며, 행 데이터 캐시는 최대 800행(약 8 MiB)까지 보관합니다. 브라우저 스크롤 트랙 높이는 최대 8,000,000 CSS px로 제한하고 이를 넘는 대용량 목록은 논리 행 위치와 스크롤 좌표를 비례 변환하되, 마우스 휠·키보드·행 번호 이동은 1행 단위 정밀도를 유지합니다.
- **필터를 바꿔도 위치 유지(`controller.js`):** 화면(`large-view.js`의 `placeOf`)은 새 필터를 보낼 때 pane의 자리를 넘깁니다. 기준 행(`state.anchor`)이 있으면 그 행과 선택의 화면 안 높이(`offset`, 보이지 않았으면 가운데), 없으면 맨 위 행입니다. 기준 행은 사용자가 고른 행(클릭·키·행 번호 이동·작업 복원)이고, 결과에서 빠져 대신 고른 행은 선택만 될 뿐 기준이 되지 않으므로 필터를 넓히면 원래 행으로 돌아갑니다. 결과가 없으면 기준과 원문 상세를 남겨 둡니다.
  - 컨트롤러는 엔진이 새 쿼리를 받은 뒤 `locate`로 기준 행을 찾아(`relocate`) 선택·원문 상세를 유지하고, 빠졌으면 그 뒤의 결과(없으면 마지막 결과)를 고른 뒤 선택이 바뀌었을 때만 `onRelocate`로 알립니다. 검색 결과에서 고른 행은 상단 로그에도 보이므로, 같은 기준을 가진 상단 로그도 검색이 대신 고른 행으로 옮깁니다(`locateLine`의 `keepAnchor`).
  - 처음 300 ms(`RELOCATE_HOLD_MS`)는 행 읽기를 미뤄 표로 판정되는 필터에서 목록 맨 앞이 잠깐 보이지 않게 하고, 그 뒤에는 맨 앞부터 보여 주며 범위 표시 자리에 `…행 찾는 중… N%`를 띄웁니다. %는 원문 상세의 `sourceStartByte`(없으면 파일 전체 크기 `state.size`)까지 읽은 비율입니다. 사용자가 스크롤하면(`continuous-view.js`의 `onUserMove`) 찾기를 멈추며(작업 복원의 위치 찾기도 포함), 기준 행을 찾던 중(`select: true`)이었다면 다시 찾아가는 `이동` 토스트를 띄웁니다. 필터 입력 중 무효화된 검색 pane은 자리를 기억해 두었다가 다음 쿼리에 넘기고, 검색어가 바뀌면 처음부터 보여 줍니다.
- **다중 조건 에디터(`conditions.js`):** 필터 패널은 조건마다 카드를 렌더링하고, 현재 선택된 카드 안으로 단일 입력 폼(`#condEditor`)을 이동시켜 사용합니다. 조건이 켜져 있고 유효한 값이 있는 조건만 코어 엔진으로 전달(`coreFilters`)하므로, 빈 조건을 추가하거나 꺼진 조건을 수정할 때는 불필요한 재쿼리가 발생하지 않습니다.
- **기능 팁과 둘러보기(`tips.js`):** 단일 `<dialog id="tipLayer">`를 사용해 첫 사용 팁은 비모달(`open` 속성, 포커스 유지 및 포인터 통과)로, 기능 둘러보기는 모달(`showModal()`)로 표시합니다. 확인한 팁 목록은 `localStorage`의 `logstudio.tips.v1`에 저장되며, macOS에서 팁 기록만 초기화하려면 앱을 종료한 뒤 `python3 desktop/reset_tips_macos.py`를 실행합니다.
- **화면 배율과 창 상태:** macOS WKWebView의 트랙패드 핀치(`gesturestart`/`gesturechange`/`gestureend`)와 Windows/Chromium의 `Ctrl+휠`을 모두 지원하며, 창 위치·크기·최대화 상태는 이동 후 400 ms 디바운스 및 앱 종료 시 `window-state.json`에 저장합니다.

## 파일 접근 및 보안 경계

- **승인된 파일만 접근:** Rust 백엔드는 OS 파일 선택 대화상자, 네이티브 드래그 앤 드롭, 명시적 CLI 인자, macOS Finder 파일 붙여넣기 일회성 권한(grant)으로 사용자가 직접 승인한 파일만 읽습니다. 프론트엔드에는 파일 경로를 직접 열어 볼 수 있는 API가 없으며, 최근 파일(`recent-files.json`), 다시 열기(`reopen_source`), 작업 상태(`work-states.json`) 모두 내부 ID(`datasetId` 또는 항목 ID)로만 통신합니다.
- **기기 기록과 외부 프로그램 실행:** 앱이 실행하는 외부 프로그램은 adb와, macOS에서 iPhone·iPad 앱 목록을 열거나 앱을 실행할 때만 쓰는 `xcode-select`·`xcodebuild`·`devicectl`뿐입니다. iPhone·iPad는 외부 프로그램 없이 OS의 Apple 기기 서비스와 기기가 이미 맺은 페어링 기록으로만 연결하며, 페어링을 새로 만들거나 바꾸지 않고 로그 서비스 하나만 엽니다. adb는 백엔드가 정해진 위치에서 찾거나 OS 파일 선택 창으로 지정한 파일만 쓰며, `adb version` 출력으로 adb인지 확인합니다. 화면은 실행 파일 경로나 명령을 보낼 수 없고, adb의 출처와 버전만 받습니다. 기기 serial은 백엔드가 마지막으로 받은 기기 목록에 있는 값만 허용하며, 셸을 거치지 않고 프로세스 인자로 넘깁니다. 기기에서 실행하는 명령은 고정된 조회와 `logcat`뿐이며, 앱 실행의 bundle ID는 마지막 앱 목록에 있고 허용 문자(영문자·숫자·`-`·`.`·`_`)만으로 된 값만 받아 `--` 뒤에 넘깁니다. 임시 기록 파일과 남은 기록도 화면에는 ID와 이름만 전달합니다.
- **확장 기능 창:** extras 빌드의 확장 창은 `extras` URI 스킴에서 페이지를 받고, 그 스킴은 확장 자신의 창이 보낸 GET 요청에만 답합니다. 지도 창만 타일 서버에 접속하며, 메인 창의 CSP와 앱 명령의 메인 창 제한은 그대로입니다. extras 빌드는 `설정 폴더 열기`에서만 앱 데이터의 `extras/` 폴더를 macOS `open`(Windows `explorer`)으로 엽니다([EXTRAS.md](EXTRAS.md#구조)).
- **메모리 및 원자적 쓰기:** 로그 원문 전체를 메모리에 복제하지 않고 필요할 때마다 128 KiB 블록 단위로 파일에서 직접 읽으며 반복 읽기는 OS 페이지 캐시를 활용합니다. 결과 내보내기와 설정 파일(`recent-files.json`, `work-states.json`, `saved-filters.json`) 저장은 모두 임시 파일에 먼저 기록한 뒤 원자적으로 교체(atomic rename)합니다.

## 빌드와 배포

CI는 Windows x64(`windows-2022`, MSVC 런타임 정적 링크)와 macOS Apple Silicon(`macos-15`) 네이티브 러너에서 테스트와 릴리스 빌드를 수행합니다. macOS 앱은 ad-hoc 서명만 하며 Developer ID 서명과 Apple 공증은 하지 않습니다.

- **아이콘 생성:** 원본은 `src-tauri/icons/icon.svg`(macOS용 정사각형)와 `icon-rounded.svg`(Windows `icon.ico` 및 `icon.png`용)입니다. 아이콘 원본을 수정한 경우 macOS에서 `python3 desktop/make_icons.py`를 실행해 아이콘 자산을 재생성합니다.
- **서드파티 라이선스 고지:** 배포 ZIP과 macOS 앱 번들에는 `desktop/THIRD_PARTY_NOTICES.md`가 포함됩니다. `Cargo.lock` 의존성을 변경한 경우 아래 명령으로 macOS arm64와 Windows x64 프로덕션 의존성 트리를 확인해 문서를 갱신합니다.

```sh
for target in aarch64-apple-darwin x86_64-pc-windows-msvc; do
  cargo tree --locked -e normal,build --target "$target" -p logstudio-desktop --prefix none --format '{p} {l}'
done | sort -u
```

### 자동 빌드와 릴리스

- `main` 브랜치 push 및 PR 시 Windows·macOS 빌드·테스트·패키징 워크플로([desktop.yml](../.github/workflows/desktop.yml))가 실행됩니다.
- 같은 워크플로는 `main` push 때 확장 기능을 넣은 빌드(`--features extras`)도 OS별로 `extras-LogStudio-macos-arm64`·`extras-LogStudio-windows-x64` 아티팩트로 올립니다. 이 아티팩트는 릴리스에 포함되지 않습니다. 두 OS 모두 PR에서도 이 빌드를 clippy로 검사합니다.
- 앱 버전(`src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `package.json` 세 곳의 버전이 일치해야 함)과 동일한 `v1.0.0` 형식의 태그를 push하면 빌드 및 체크섬 검증 후 GitHub Releases에 Windows·macOS ZIP과 각각의 `.zip.sha256` 파일이 자동 게시됩니다.

### macOS에서 한 번에 빌드하고 실행

```sh
sh build-run-macos.command
```

- `rustup`으로 설치한 Rust(`rust-toolchain.toml`의 1.99.0), Xcode Command Line Tools, Python 3.9 이상이 필요합니다.
- `--locked` 릴리스 바이너리를 빌드해 `artifacts/desktop/LogStudio-local.app`으로 패키징한 뒤 실행합니다. 직전 빌드는 `LogStudio-local-backup-<고유번호>.app` 한 개만 백업으로 남기고 오래된 백업은 정리합니다.
- 중복 실행 방지를 위해 빌드 중에는 `artifacts/desktop/.logstudio-local-build.lock` 잠금 폴더를 사용합니다.

## 검사

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo clippy --locked -p logstudio-desktop --all-targets --features extras -- -D warnings
cargo test --locked --workspace
npm test
npm run check
python3 -m unittest discover -s tests -p 'test_*.py'
```

Node 테스트는 외부 패키지 설치(`npm install`) 없이 Node.js 22 이상 내장 테스트 러너로 실행됩니다. `dist/`에 JS 파일을 추가한 경우 `package.json`의 `check` 스크립트 대상에도 함께 추가하세요.

### 실제 WebView 자동 검사

`--features qa` 빌드에 포함된 내장 WebDriver로 실제 macOS WKWebView를 구동해 검증합니다(프로덕션 빌드에는 포함되지 않음).

```sh
CARGO_TARGET_DIR=target/qa cargo build --release --locked -p logstudio-desktop --features qa
python3 tests/e2e/run.py target/qa/release/logstudio-desktop
```

자세한 시나리오 목록과 실행 옵션은 [tests/e2e/README.md](../tests/e2e/README.md)를 참고하세요.

### 실제 기기·에뮬레이터 검사

`core/tests/ios_device.rs`는 이 컴퓨터를 신뢰한 iPhone·iPad(USB나 Wi-Fi 연결)로 제품의 iOS 기기 연결을 검증하며, 평소 테스트에서는 건너뜁니다. 시작한 뒤의 줄만 기록되는지, 모든 줄이 iOS 로그로 읽히는지(이어지는 줄 제외)와 초당 줄 수를 확인하고, 케이블을 뽑았다 꽂는 재연결 검사는 사람이 직접 해야 합니다. Finder에서 Wi-Fi 보기를 켠 기기는 케이블을 뽑아도 Wi-Fi로 이어지므로, 케이블 재연결을 확인하려면 이 설정을 끄고 합니다. 여러 대가 연결되어 있으면 `LOGSTUDIO_IOS_UDID`로 고릅니다. 개발용 서명 앱 실행 검사는 Xcode와 기기의 개발용 서명 앱이 필요하며, `LOGSTUDIO_IOS_APP`에 그 앱의 bundle ID를 지정하면 기록 중에 앱을 다시 실행해 그 앱 프로세스의 줄에 `<private>`가 없는지 확인합니다. 지정한 앱은 종료된 뒤 다시 실행됩니다.

```sh
cargo test -p logstudio-core --test ios_device -- --ignored --test-threads 1 --nocapture
LOGSTUDIO_IOS_APP=<bundle ID> cargo test -p logstudio-core --test ios_device a_developer_build -- --ignored --nocapture
```

`core/tests/capture_device.rs`는 연결한 기기나 에뮬레이터로 제품의 adb 연결을 검증하며, 평소 테스트에서는 건너뜁니다(`#[ignore]`). 시작 경계, adb 서버 재시작 후 이어 붙이기, 재부팅 후 기록, 설치된 adb 탐지를 확인하며, 기기 쪽 로그는 테스트가 실행 중에 직접 찍은 확인용 줄로만 판정합니다.

```sh
LOGSTUDIO_ADB=~/Library/Android/sdk/platform-tools/adb LOGSTUDIO_SERIAL=emulator-5554 \
  cargo test -p logstudio-core --test capture_device -- --ignored --test-threads 1 --nocapture
```

### 브라우저 미리보기

`src-tauri/examples/web_preview`는 `dist/` 화면을 일반 웹 브라우저(`127.0.0.1`)에 띄우는 개발 전용 로컬 서버입니다. 백엔드는 실제 Rust 엔진을 그대로 사용하며, 브라우저 개발자 도구로 접근성이나 스타일을 빠르게 점검할 때 유용합니다.

```sh
cargo run -p logstudio-desktop --example web_preview -- --port 8790 path/to/synthetic.log
```

### 성능 측정

`bench/` 워크스페이스 크레이트의 바이너리로 인덱싱·분석·필터·검색·따라가기 성능을 측정합니다. 상세 사용법은 [bench/README.md](../bench/README.md)에, 측정 결과는 [LARGE_LOG_VALIDATION.md](LARGE_LOG_VALIDATION.md)에 정리되어 있습니다.
