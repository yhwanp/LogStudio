# COVESA DLT 샘플

COVESA 저장소에 공개된 테스트용 DLT 파일 원본입니다. 실제 도구가 생성한 DLT 파일을 LogStudio가 정확히 파싱하는지 검증하는 데 사용하며, 테스트 전용이라 앱 배포 패키지에는 포함되지 않습니다. 관련 테스트는 `core/tests/covesa.rs`와 `core/tests/dlt_decoder.rs`(`headerless_messages_are_recognised`)에 있습니다.

| 파일 | 출처 | 크기 | SHA-256 | 검증 항목 |
| --- | --- | ---: | --- | --- |
| `dlt-daemon-testfile_extended.dlt` | [COVESA/dlt-daemon](https://github.com/COVESA/dlt-daemon) `47d1def` `tests/testfile_extended.dlt` | 5,012 B | `eabd390f…0a7b` | 버전 1 메시지 111개(다양한 verbose 인자 타입, control 요청, non-verbose, 타입 길이가 없는 BOOL). 렌더링 결과(`dlt-daemon-testfile_extended.txt`, UTC), `dlt_parse` 결과와의 일치 여부, `.dlt`·`.txt` 내보내기 검증 |
| `dlt-daemon-testfile-v2.dlt` | 같은 저장소 `tests/testfile-v2.dlt` | 128 B | `5492775a…01c5` | 버전 2 전용 파일 거부 확인 |
| `dlt-viewer-testfile.dlt` | [COVESA/dlt-viewer](https://github.com/COVESA/dlt-viewer) `029a485` `qdlt/tests/testfile.dlt` | 71 B | `7a0a613b…7d1984` | 스토리지 헤더가 없는 버전 2 단일 메시지 거부 및 안내 문구 확인 |

`dlt-daemon-testfile_extended.txt`는 LogStudio가 렌더링한 기대 출력(golden) 파일입니다. 최초 생성 시 원본 메시지 바이트 및 `dlt_parse` 파싱 결과와 대조해 검증했습니다. 렌더링 형식을 의도적으로 변경한 경우에만 `LOGSTUDIO_WRITE_FIXTURE=1 cargo test -p logstudio-core --test covesa`로 파일을 갱신한 뒤 테스트를 다시 실행해 확인합니다.

## 라이선스

세 `.dlt` 파일과 이로부터 생성된 `dlt-daemon-testfile_extended.txt`는 원본 저장소의 라이선스인 Mozilla Public License 2.0(MPL-2.0)을 따릅니다. 바이너리 파일 내부에는 라이선스 고지를 직접 넣을 수 없어 이 문서에 명시합니다. 이 폴더를 제외한 저장소의 나머지 코드는 루트 [LICENSE](../../../LICENSE)의 MIT 라이선스를 따릅니다.

```
This Source Code Form is subject to the terms of the Mozilla Public License,
v. 2.0. If a copy of the MPL was not distributed with this file, You can
obtain one at https://mozilla.org/MPL/2.0/.
```
