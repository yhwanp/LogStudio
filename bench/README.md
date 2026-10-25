# 엔진 성능 측정 (`bench/`)

합성 로그를 생성하고 코어 엔진 API를 직접 호출해 소요 시간과 결과 건수를 측정하는 벤치마크 도구 모음입니다. 앱 셸이나 WebView는 거치지 않으며, Cargo workspace 멤버로 포함되어 CI에서 빌드 가능 여부만 검증합니다(배포 패키지에는 포함되지 않음). 측정 조건과 결과 기록은 [LARGE_LOG_VALIDATION.md](../docs/LARGE_LOG_VALIDATION.md)에 있습니다.

```sh
cargo build --release --locked -p logstudio-bench
target/release/field-filters-gen 512 /tmp/a-512.log
target/release/field-filters /tmp/a-512.log current 2
```

생성기는 지정한 크기가 같으면 항상 동일한 내용을 생성합니다. 대용량 입력 파일은 저장소 바깥의 임시 폴더에 생성하고 측정이 끝나면 삭제합니다.

## 입력 생성기

| 바이너리 | 인자 | 생성 내용 |
| --- | --- | --- |
| `field-filters-gen` | `<MiB> <path> [wide]` | Android 2 : iOS 1 비율, 한글·이모지 포함, 1,024행마다 `RarE_NEEDLE_<i>`, 262,144행마다 희귀 행(태그 `RareFieldTag`, PID 4242, TID 4243) 삽입. `wide` 옵션은 태그 400종·iOS 컴포넌트 60종(하위 9종 조합 총 540종)·PID 80개·TID 2,000개의 편향 분포로 생성 |
| `continuation-gen` | `<MiB> <path>` | 약 1/3이 멀티라인(스택 트레이스·JSON 본문)인 Android/iOS 로그, 빈 줄·손상 UTF-8 줄·`rare_needle_` 포함 |
| `multi-value-gen` | `<MiB> <path>` | Android PID 64개·태그 40종, iOS PID 8개·다중 태그 행 및 일부 스택 트레이스 |
| `multi-file-gen` | `<MiB> <files> <dir>` | `<dir>/big-<k>.log` 생성(이미 있으면 건너뜀). 파일별 고유 표식 `RarE_NEEDLE_F<k>_` 삽입, 첫 파일은 줄바꿈 없이 끝나고 둘째 파일은 CRLF 줄바꿈 사용 |
| `follow gen` | `<path> <MiB>` | 실시간 따라가기 측정용 기본 로그 |
| `dlt-gen` | `<MiB> <path.dlt> [android\|ios\|stress] [--twin <path.log>]` | 합성 DLT 파일 생성. `android`: 문자열·정수 인자, 레벨 분포(V 20·D 30·I 35·W 8·E 5·F 2%), ECU 3 × APID 40 × CTID 200, 선택 헤더 필드 전체, 10% big-endian, 1,024개마다 `RarE_NEEDLE_<i>`. `ios`: 대괄호 컴포넌트 문자열·BOOL·f64, 약 30% 한글·이모지, 5% RAWD(16–64 B). `stress`: `android` 조건에 non-verbose 15%·control 5%·4 KiB RAWD 1%, 약 10 MiB마다 1–8 KiB 비정상 바이트 구간 및 파일 끝 잘린 메시지 추가. `--twin` 지정 시 동일한 행(UTC)을 텍스트 로그로도 함께 저장 |

## 측정 바이너리

| 바이너리 | 인자 | 측정 항목 |
| --- | --- | --- |
| `field-filters` | `<path> <label> [reps]` | 인덱싱, 분석, 레벨·태그·PID·TID·제외·시간 필터, 빈출 검색어 + 레벨 E 복합 필터(검색에 일치한 행마다 파싱)의 쿼리 완료 시간 및 중간 페이지 조회. 필드 메타데이터 표 사용 여부별 교차 측정 및 결과 일치 검증. `PERF_FILTERS=wide`는 `wide` 입력용 필터, `conditions`는 다중 포함·제외 조건, `times`는 시간 조건 1–32개 측정(힙 통계는 macOS 전용) |
| `dlt` | `<path> [label]` | DLT 파일 또는 동일 내용 텍스트 로그(`dlt-gen --twin`)를 엔진 API로 열어 첫 페이지·인덱싱·분석, 레벨·APID·ECU 제외·1분 시간 필터, 희귀 문자열·빈출 단어·정규식 검색(첫 결과 및 완료, 각 3회), 중간 페이지, `locate`, 검색 취소 시간 측정 |
| `dlt-render` | `<path.dlt> [rounds]` | 첫 256 MiB를 128 KiB 이하 블록으로 나누어 단일 스레드에서 단위 분할·행 렌더링(시스템 시간대)·필드 파싱 속도 측정(입력 MB/s, 행/s, 블록당 렌더링 p50·p99, 텍스트/원본 크기 비율) |
| `field-collect` | `<path> [rounds]` | 첫 256 MiB 대상 단일 스레드 분석 패스의 행당 처리 비용(필드 수집 유무 비교) |
| `continuation` | `<path>` | 멀티라인 로그 열기·검색·페이지 조회 시간 및 결과 다이제스트 확인 |
| `dlt-mask` | `<path.dlt>` | 첫 256 MiB 대상 단일 스레드에서 `Filter::dlt_mask` 단독 실행 시 메시지당 소요 시간(ns, 조건 1·8·32개) |
| `continuation-mask` | `<path>` | 첫 256 MiB 대상 단일 스레드 블록별 필드 판정 시간 |
| `multi-value` | `<path> [single\|all]` | 태그·PID·TID 다중 값 필터 쿼리 시간(4 워커, 3회 중 최소·최대) |
| `multi-value-mask` | `<path> [single\|all]` | 다중 값 필터의 블록별 필드 판정 시간 |
| `multi-value-predicate` | `<path> [single\|all]` | `Filter::fields_match` 단독 실행 시 행당 소요 시간(ns) |
| `time-predicate` | `<path>` | 첫 64 MiB 대상 단일 스레드에서 시간 조건 1–32개(포함·제외) 및 비교용 태그 조건의 `Filter::fields_match` 행당 소요 시간(ns) |
| `message-filter` | `<path> [runs]` | `field-filters-gen` 로그를 엔진 API로 열어 집계까지 마친 뒤, 상단 pane의 메시지 조건(보기·제외, 고정 문자열·정규식, 복합 조건) 쿼리 및 0건 조건 확인 완료 시간을 레벨·1분 시간 필터·필드 없는 검색과 비교 |
| `multi-file` | `[--each] <path>...` | 여러 파일을 병합해 열었을 때의 첫 행 표시·인덱싱·검색·내보내기·취소·닫기 시간 |
| `file-order` | `<빈 임시 폴더>` | 다중 파일 선택 시 `order::arrange` 정렬에 추가되는 시간과 첫 행 표시 시간 |
| `follow` | `grow\|idle\|poll …` | 파일 증가분 이어 읽기(따라가기)와 전체 다시 열기 비교, 유휴 상태 CPU 사용량, 폴링 1회 비용(소스 상단 주석 참고) |
| `follow-race` | `<path> <writer ms> <attempts>` | 다른 스레드가 파일 끝에 기록하는 중 열기를 시도할 때의 실패 횟수 |
| `follow-reopen` | `<path>` | 동일한 로그를 다시 열 때의 인덱싱·쿼리·레벨 분석 완료 시간 |
| `zero-conditions` | `<path> [methods] [rounds]` | 일치하는 줄이 없는 조건(0건 조건) 탐지 성능 측정. 조건 1·8·32개 조합에 대해 `table`(레벨·필드 표 판정), `first`(조건별 첫 일치 중단 스캔), `joint`(표로 미확정된 조건 일괄 스캔), `count`(조건별 카운트 쿼리, 최대 8개) 비교 및 결과 일치 검증. `rows`(단일 스레드 행 판정 ns), `beside`(일반 쿼리와의 동시 실행·양보·취소), `engine`(상단 pane 쿼리에서의 조건 확정·재실행·단일 조건 변경·교체 시간), `overhead`(조건 확인 유무에 따른 상·하단 쿼리 완료 시간 비교) 지원 |
| `read-lock` | `<MiB> <dir> [runs] [sections]` 또는 `--file <path> [runs] [sections]` | `sections`(쉼표 구분, 기본 전체): `reads`는 블록 읽기 경로의 메타데이터 검사 비용과 1·2·4·8 스레드 처리량, `engine`은 열기·인덱싱·집계 전 레벨 필터·집계, 검색 3종, 표로 판정되는 필터, 흔한 검색어 + 레벨 필터(Error 행에 없는·있는 검색어), 시간 범위 필터의 첫 결과·완료 시간과 실행 중인 검색 취소 시간, `restore`는 작업 복원처럼 파일과 함께 시작한 필드 필터의 첫 행·건수 확정·레벨 집계 완료 시간, `search`는 집계 뒤 검색 3종과 표로 판정되는 필터 3종을 각각 5회씩 반복한 완료 시간. OS 파일 캐시보다 큰 파일(24 GiB Mac의 8 GiB 등)은 매번 디스크에서 읽은 값. 입력은 `<dir>/bench-<MiB>.log`에 생성(이미 있으면 재사용) |

## 보조 스크립트 (`scripts/`)

`scripts/` 폴더의 셸 스크립트는 macOS의 `/usr/bin/time -l`을 이용해 최대 메모리 사용량(peak RSS)을 함께 기록합니다.

- `field-filters.sh <dir> <rounds> <reps for 2 GiB> <label>=<release dir>...`: 512 MiB·2 GiB 기본 및 `wide` 입력을 생성하고 빌드별 프로세스를 교대로 실행합니다. `field-filters-summary.py <dir>/out`으로 빌드별 측정 범위와 건수 일치 여부를 요약합니다.
- `follow-grow.sh`, `follow-idle.sh`, `follow-race.sh`, `follow-reopen.sh`: 따라가기 측정을 A/B 빌드로 번갈아 반복합니다(인자는 각 스크립트 첫 줄 참고).
- `zero-conditions.sh <dir> <processes> [release dir]`: 텍스트(`field-filters-gen`)와 DLT(`dlt-gen android`) 512 MiB·2 GiB 입력을 생성해 `zero-conditions`를 별도 프로세스로 반복 실행하고, `zero-conditions-summary.py <dir>/out`으로 요약합니다.

## 이전 커밋과 비교 (`baseline.sh`)

```sh
bench/baseline.sh <commit> <저장소 밖 폴더> field-filters field-filters-gen field-collect
```

지정한 커밋의 `core`와 workspace 설정을 외부 폴더로 내보내고 현재 `bench/` 소스를 복사해 `<폴더>/target/release`에 벤치마크 바이너리를 빌드합니다(해당 커밋의 코어에 없는 API를 쓰는 바이너리는 제외). 빌드 후 두 바이너리를 동일 입력에 대해 별도 프로세스로 번갈아 실행해 비교합니다.

```sh
bench/scripts/field-filters.sh /tmp/ff 2 1 base=<폴더>/target/release new=target/release
```
