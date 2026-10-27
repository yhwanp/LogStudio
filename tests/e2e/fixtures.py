"""Fully synthetic fixtures for the scenarios. Never put real logs here.

Each writer creates its files in the given folder and returns what the
scenario needs to check them (file names, row spans).
"""
import json
import shutil
import struct
from pathlib import Path


def ui(root: Path):
    """ui.log: 3,000 Android threadtime rows over 6 tags and 2 PIDs; row 50
    is a 200 KB line so the detail pages through it."""
    levels = 'IDIWDIEVID'
    tags = ['ActivityManager', 'RenderThread', 'ConnectivityService', 'ExampleApp', 'WifiMonitor', 'NetworkStack']
    msgs = ['Starting activity: com.example.demo/.MainActivity', 'Frame rendered in 12 ms', 'Network connected: WIFI',
            'Slow response: 420 ms', 'Request failed: timeout after 3000 ms', 'Cache lookup complete',
            '화면 준비 완료 • synthetic demo']
    out = []
    for i in range(3000):
        pid, tid = ('2983', '4241') if i % 3 == 0 else ('12540', '12610')
        msg = msgs[i % len(msgs)]
        if i == 49:
            msg = 'LONGLINE ' + 'abcdefghij' * 20000 + ' END'
        out.append(f'06-18 19:{24 + i // 1200:02d}:{(i // 20) % 60:02d}.{(i % 20) * 50:03d}  {pid} {tid} {levels[i % 10]} {tags[i % 6]:<20}: {msg}')
    (root / 'ui.log').write_text('\n'.join(out) + '\n', encoding='utf-8')
    return out


def days(root: Path):
    """days.log: one row every 30 minutes from 06-18 00:00 to 06-19 23:30."""
    out = []
    for i in range(96):
        day, minutes = 18 + i // 48, (i % 48) * 30
        out.append(f'06-{day:02d} {minutes // 60:02d}:{minutes % 60:02d}:00.000  1000 1001 {"IWE"[i % 3]} DayTag              : row {i} WIFI')
    (root / 'days.log').write_text('\n'.join(out) + '\n', encoding='utf-8')


def lists(root: Path):
    """lists.log: Android rows rotate 3 PIDs and 4 tags; every fifth row is iOS
    with several component tags."""
    tags = ['ActivityManager', 'MyApp', 'chatty', 'Choreographer']
    out = []
    for i in range(60):
        if i % 5 == 4:
            pid, tid, body = ('3870', '481516', '[Memory] [SDK_UI] heap used') if i % 2 else ('3871', '481517', '[Network] request')
            out.append(f'2026-09-15 13:32:{i:02d}.134000 Runner[{pid}:{tid}] {body} {i}')
        else:
            pid = ['2983', '3120', '4000'][i % 3]
            out.append(f'06-18 19:24:{i:02d}.000  {pid} {5000 + i % 4} {"IDWE"[i % 4]} {tags[i % 4]:<16}: synthetic message {i} 화면 준비')
    (root / 'lists.log').write_text('\n'.join(out) + '\n', encoding='utf-8')
    return out


# Every Android format the parser recognizes, with continuation and malformed rows.
FORMATS = """--------- beginning of main
06-18 19:24:01.123 D/AlertsService( 2983): Sending update
  {"body": "안내문구3"}
06-18 19:24:02.500 E/AndroidRuntime( 2983): FATAL EXCEPTION: main
\tat com.example.Main.run(Main.java:10)
2026-06-18 19:24:03.000  2983-4241  AlertsService           com.example.app                      W  Studio 한글 🙂
                                                                                                      second line
2026-06-18 19:24:04.250  1000-1000  ActivityManager         system_server                        I  Start proc
06-18 19:24:05.000  1000: 2983  4241 I AlertsService: uid row
I/ActivityManager( 1000): brief row
  brief continuation
06-18 19:24:06.000 X/Broken( 1): malformed
  tail of malformed
09-21 12:07:30.386  2983  4241 E Legacy: threadtime
2026-06-18 19:24:07.000 Runner[1:2] [INFO] [Swift] ios row
2026-06-18 19:24:08.000  4321-4321  알림🙂                     com.example.app                      V  유니코드 태그
"""


def formats(root: Path):
    (root / 'formats.log').write_text(FORMATS, encoding='utf-8')


def startup(root: Path):
    """startup.log: threadtime, -v year, -v nsec and iOS rows and one raw stack line."""
    (root / 'startup.log').write_text(
        "09-21 12:07:30.386  1234  5678 I Classic: threadtime\n"
        "2026-09-21 12:07:31.386  1234  5678 W Dated: year\n"
        "09-21 12:07:32.386123456  1234  5678 E Nano: nsec\n"
        "2026-09-30 21:55:53.776261 Runner[7:9] [INFO] [SDK_UI] ios body\n"
        "    at com.example.Foo.bar(Foo.java:1)\n", encoding='utf-8')


def mojibake(root: Path):
    """mojibake.log: iOS rows whose UTF-8 was read as Mac OS Roman, and a plain row."""
    garble = lambda s: s.encode('utf-8').decode('mac_roman')
    (root / 'mojibake.log').write_text(
        f"2026-09-30 21:55:53.776261 Maps[7:9] [{garble('인사')}] "
        f"Greeting(text : {garble('안녕하세요')}, lang : ko, city : {garble('대전')}) {garble('🙂')}\n"
        "2026-09-30 21:55:53.776262 Maps[7:9] [Cache] café “quoted” – 5 °C\n", encoding='utf-8')


def combined(root: Path):
    """first/second/third.log: first ends without a line ending, second uses CRLF."""
    files = {
        'first.log': b'06-18 10:00:00.000   100   200 I Alpha: first-1\n06-18 10:00:01.000   100   200 E Alpha: first-2',
        'second.log': b'06-18 11:00:00.000   300   400 W Beta: second-1 WIFI\r\n06-18 11:00:01.000   300   400 I Beta: second-2\r\n',
        'third.log': b'06-18 12:00:00.000   500   600 E Gamma: third-1 wifi\n',
    }
    for name, data in files.items():
        (root / name).write_bytes(data)


def order(root: Path):
    """a/b/c.log hold one day each, newest first by name; logcat_* have no times."""
    files = {
        'a.log': '06-18 10:00:00.000   300   300 E New: a1 newest\n',
        'b.log': '06-17 10:00:00.000   200   200 I Mid: b1 middle\n',
        'c.log': '--------- beginning of main\n06-16 10:00:00.000   100   100 I Old: c1 oldest\n06-16 10:00:01.000   100   100 W Old: c2 oldest\n',
        # No log times: natural name order decides (2 before 10).
        'logcat_10.log': 'I/Ten( 1): ten\n',
        'logcat_2.log': 'I/Two( 2): two\n',
    }
    for name, text in files.items():
        (root / name).write_text(text, encoding='utf-8')


def records(root: Path):
    """record.log: multi-line entries (an iOS JSON body with CRLF, an Android
    stack trace, a malformed stamped row with a tail, a long entry over two
    64 KiB detail windows, a 21 KB row with a word deep inside). Returns the
    row spans."""
    out, meta = [], {}

    def add(text, br='\n'):
        out.append((text, br))
        return len(out)
    add('preamble before any log')
    first = add('2026-09-30 21:55:54.351980 [DEBUG] [AlertsService] [', '\r\n')
    for row in ['  {', '    "body": "안내문구3 🙂",', '    "schedule_window_at": "2026-10-02T09:00:00.000Z",', '    "snooze_threshold": "today"', '  }']:
        add(row, '\r\n')
    meta['ios'] = [first, add(']')]
    first = add('09-21 12:07:30.386  1234 5678 E AndroidRuntime: FATAL EXCEPTION: main')
    for k in range(4):
        add(f'\tat com.example.app.Worker{k}.run(Worker.java:{40 + k})')
    meta['android'] = [first, add('\tat java.lang.Thread.run(Thread.java:1012)')]
    meta['single'] = add('09-21 12:07:30.400  1234 5678 I ActivityManager: Displayed com.example/.Main')
    meta['malformed'] = add('2026-09-30 21:55:55.100 Runner[3870:bad] malformed')
    meta['orphanTail'] = add('  detail of the malformed row')
    for i in range(1500):
        add(f'09-21 12:08:{(i // 25) % 60:02d}.{(i % 25) * 40:03d}  2983 4241 {"IDWIE"[i % 5]} Tag{i % 6}: message {i} • 합성')
        if i % 50 == 7:
            add(f'    continued payload {i}')
    first = add('2026-09-30 21:56:00.000001 [ERROR] [BigBody] [')
    for k in range(2500):
        add(f'    {{"index": {k}, "value": "긴 본문 {k:05d} 🙂"}},')
    meta['big'] = [first, add(']')]
    for i in range(500):
        add(f'09-21 12:09:{(i // 25) % 60:02d}.{(i % 25) * 40:03d}  2983 4241 I Tail: after {i}')
    meta['needle'] = add('09-21 12:10:00.000  2983 4241 I LongTail: ' + 'abcdefghij' * 2000 + ' NEEDLE ' + 'klmnopqrst' * 100)
    (root / 'record.log').write_bytes(''.join(t + b for t, b in out).encode())
    meta['rows'] = len(out)
    meta['lines'] = [t for t, _ in out]
    return meta


def follow_row(n, tag='App', level='I', text=None):
    return f"06-18 19:{n // 3600 % 60:02}:{n // 60 % 60:02}.{n % 1000:03}  1{n % 7} 2{n % 5} {level} {tag}: {text or f'row {n} 한글 🙂'}"


def live(root: Path):
    """live.log: 299 complete rows and an unterminated 300th, for following."""
    path = root / 'live.log'
    path.write_text(''.join(follow_row(n) + '\n' for n in range(1, 300)) + follow_row(300, text='partial'), encoding='utf-8')
    return path


GROW_TAGS = ['ActivityManager', 'RenderThread', 'WifiMonitor', 'ExampleApp', 'NetworkStack']


def grow_row(i, mark=''):
    if i % 3 == 2:
        return f'2026-06-18 19:{24 + i // 1200:02d}:{(i // 20) % 60:02d}.{i % 1000:06d} Runner[3870:4401] [INFO] [SDK_UI] [GalleryManager] 화면 준비 🙂 {mark}{i}'
    return f'06-18 19:{24 + i // 1200:02d}:{(i // 20) % 60:02d}.{(i % 20) * 50:03d}  2983 4241 {"IDWEI"[i % 5]} {GROW_TAGS[i % 5]:<16}: synthetic message {mark}{i} timeout={i % 7}'


def grow(root: Path, rows=3000, mark=''):
    """grow.log: Android/iOS rows that a scenario rewrites or appends to."""
    path = root / 'grow.log'
    path.write_text(''.join(grow_row(i, mark) + '\n' for i in range(rows)), encoding='utf-8')
    return path


def scaled(root: Path):
    """scaled.log (~28 MB): 400,000 rows, so the lists' native scrollbar is
    scaled (400,000 rows of 28 px exceed its 8,000,000 px). Row n is
    scaled_row(n - 1); every fourth row has `timeout`."""
    with open(root / 'scaled.log', 'wb') as f:
        chunk = ''.join(scaled_row(i) + '\n' for i in range(1000)).encode()
        for _ in range(400):
            f.write(chunk)


def scaled_row(i):
    msgs = ['Frame rendered in 12 ms', 'Network connected: WIFI', 'Request failed: timeout', 'Cache lookup complete']
    i %= 1000
    return f'06-18 19:24:{(i // 20) % 60:02d}.{(i % 20) * 50:03d}  2983 4241 {"IDIW"[i % 4]} ScaledTag: {msgs[i % 4]} #{i}'


def rows_10m(root: Path):
    """rows-10m.log (~400 MB): 10,000,050 rows for 8-digit row numbers, and
    small.log with the same rows. `PolishTail` marks row 10,000,026."""
    levels = 'IDIWDIEVID'
    tags = ['ActivityManager', 'RenderThread', 'WifiMonitor', 'ExampleApp']
    msgs = ['Frame rendered in 12 ms', 'Network connected: WIFI', 'Request failed: timeout', '화면 준비 완료 🙂']

    def block(n, needle):
        rows = [f'06-18 19:24:{(i // 20) % 60:02d}.{(i % 20) * 50:03d}  {2983 + i % 3} {4241 + i % 7} {levels[i % 10]} {tags[i % 4]}: {msgs[i % 4]}' for i in range(n)]
        rows[n // 2] += f' {needle}'
        return ('\n'.join(rows) + '\n').encode()
    chunk = block(1000, 'PolishNeedle')
    with open(root / 'rows-10m.log', 'wb') as f:
        for _ in range(10000):
            f.write(chunk)
        f.write(block(50, 'PolishTail'))
    (root / 'small.log').write_bytes(block(3000, 'PolishNeedle'))


def write_json(path: Path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=1) + '\n', encoding='utf-8')


DLT_BASE = 1_773_480_413  # 2026-03-14 09:26:53 UTC


def _dlt_id(text):
    return text.encode('ascii')[:4].ljust(4, b'\0')


def _dlt_message(seconds, micros, ecu, ext, payload, count=0, session=4242, uptime=None):
    """One storage-header message; `ext` is (msin, noar, apid, ctid) or None."""
    htyp = 0x20 | 0x04 | 0x08 | (0x10 if uptime is not None else 0) | (0x01 if ext else 0)
    extra = _dlt_id(ecu) + struct.pack('>I', session) + (struct.pack('>I', uptime) if uptime is not None else b'')
    if ext:
        msin, noar, apid, ctid = ext
        extra += struct.pack('>BB', msin, noar) + _dlt_id(apid) + _dlt_id(ctid)
    standard = struct.pack('>BBH', htyp, count & 0xff, 4 + len(extra) + len(payload)) + extra
    return b'DLT\x01' + struct.pack('<II', seconds, micros) + _dlt_id(ecu) + standard + payload


def _dlt_text(text):
    data = text.encode('utf-8') + b'\0'
    return struct.pack('<IH', 0x200 | 0x8000, len(data)) + data


def dlt(root: Path):
    """vehicle.dlt: 36 verbose log messages over three ECUs, five APIDs and
    every level, Korean text, a control response, a message without an
    extended header and 5 bytes that are no message. early.dlt: one message
    an hour before. Returns the expected rows of vehicle.dlt."""
    levels = [3, 4, 2, 1, 5, 6]  # warn, info, error, fatal, debug, verbose
    names = {1: 'Fatal', 2: 'Error', 3: 'Warn', 4: 'Info', 5: 'Debug', 6: 'Verbose'}
    apps = [('NAV', 'RTE', 'Route recalculated'), ('HMI', 'UI', '경로 안내 시작 🙂'), ('AUD', 'MIX', 'Volume limiter active'),
            ('SYS', 'BOOT', 'Watchdog check'), ('NAV', 'GPS', 'Fix acquired')]
    ecus = ['ECU1', 'HU', 'TCU']
    out, rows = bytearray(), []
    for i in range(36):
        apid, ctid, text = apps[i % len(apps)]
        level, ecu = levels[i % len(levels)], ecus[i % len(ecus)]
        message = f'{text} {i}'
        out += _dlt_message(DLT_BASE + i, i * 1000, ecu, (1 | level << 4, 1, apid, ctid), _dlt_text(message), count=i, uptime=123_450_000 + i * 10)
        rows.append({'line': len(rows) + 1, 'seconds': DLT_BASE + i, 'micros': i * 1000, 'ecu': ecu, 'apid': apid, 'ctid': ctid,
                     'level': names[level], 'message': message, 'uptime': f'{12345 + i // 1000}.{(i * 10) % 10000:04d}'})
        if i == 9:
            out += _dlt_message(DLT_BASE + i, 500_000, 'ECU1', (3 << 1 | 2 << 4, 0, 'DA1', 'DC1'), struct.pack('<IB', 3, 0) + bytes([7, 1]), count=i)
            rows.append({'line': len(rows) + 1, 'control': True})
        if i == 19:
            out += _dlt_message(DLT_BASE + i, 600_000, 'TCU', None, struct.pack('<I', 4096) + b'AB\x00', count=i)
            rows.append({'line': len(rows) + 1, 'nonverbose': True})
        if i == 29:
            out += b'\x00\x13zz\xff'
            rows.append({'line': len(rows) + 1, 'issue': True})
    (root / 'vehicle.dlt').write_bytes(bytes(out))
    (root / 'early.dlt').write_bytes(_dlt_message(DLT_BASE - 3600, 0, 'ECU1', (1 | 4 << 4, 1, 'EARL', 'Y'), _dlt_text('earlier file')))
    return rows


COVESA_EXTENDED = 'dlt-daemon-testfile_extended.dlt'
COVESA_V2 = 'dlt-daemon-testfile-v2.dlt'
COVESA_HEADERLESS = 'dlt-viewer-testfile.dlt'


def covesa(root: Path):
    """The public COVESA samples in tests/fixtures/covesa: a dlt-daemon file
    of 111 version 1 messages, one of version 2 only, and dlt-viewer's
    version 2 message without a storage header."""
    source = Path(__file__).resolve().parents[1] / 'fixtures' / 'covesa'
    for name in (COVESA_EXTENDED, COVESA_V2, COVESA_HEADERLESS):
        shutil.copyfile(source / name, root / name)


def track(root: Path):
    """track.log: 1,200 Android threadtime rows. From row 10, every third
    row is a NavSample position (`onPositionChanged` and a JSON fix in
    degrees times 1e7) along a synthetic loop; rows 301 and 601 carry the
    marker with a fix that has no number. Returns the points as (row,
    latitude, longitude)."""
    import math
    rows, points = [], []
    for i in range(1200):
        line = i + 1
        time = f'10-25 09:{i // 600:02d}:{(i // 10) % 60:02d}.{(i % 10) * 100:03d}'
        if line in (301, 601):
            rows.append(f'{time}  4100  4120 W NavSample           : onPositionChanged {{"fix":{{"lat":"pending"}}}}')
        elif i >= 9 and i % 3 == 0:
            k = len(points)
            a = k / 60 * math.pi
            lat, lon = round((52.52 + 0.004 * math.sin(a)) * 1e7), round((13.405 + 0.006 * math.cos(a) + k * 0.00002) * 1e7)
            points.append((line, lat / 1e7, lon / 1e7))
            rows.append(f'{time}  4100  4120 D NavSample           : onPositionChanged {{"fix":{{"lat":{lat},"lon":{lon},"alt":0}},"seq":{k}}}')
        else:
            rows.append(f'{time}  4100  4121 {"ID"[i % 2]} ExampleApp          : synthetic step {i}')
    (root / 'track.log').write_text('\n'.join(rows) + '\n', encoding='utf-8')
    return points


def track_second(root: Path):
    """track-second.log and track-both.log: 600 Android threadtime rows
    each. From row 5, every fourth row is a GpsSample position in
    `name : value` text (`LocationReport` and degrees); track-both.log
    also has a NavSample position (as in track.log) every fifth row from
    row 300. Returns how many points each log draws: the second source's
    in track-second.log, then the first source's in track-both.log."""
    import math
    counts = []
    for name, first_from in (('track-second.log', None), ('track-both.log', 300)):
        rows, gps, nav = [], 0, 0
        for i in range(600):
            line = i + 1
            time = f'10-26 14:{i // 600:02d}:{(i // 10) % 60:02d}.{(i % 10) * 100:03d}'
            if first_from and line >= first_from and line % 5 == 0:
                lat, lon = round((52.52 + 0.003 * math.sin(nav / 20)) * 1e7), round((13.405 + nav * 0.00003) * 1e7)
                rows.append(f'{time}  4100  4120 D NavSample           : onPositionChanged {{"fix":{{"lat":{lat},"lon":{lon},"alt":0}},"seq":{nav}}}')
                nav += 1
            elif line >= 5 and line % 4 == 0:
                a = gps / 30 * math.pi
                rows.append(f'{time}  4200  4230 I GpsSample           : LocationReport(source : simulated, time : {1761487200000 + i * 100}, '
                            f'lat : {52.515 + 0.005 * math.sin(a):.6f}, lon : {13.40 + 0.007 * math.cos(a):.6f}, accuracy : 5.000000)')
                gps += 1
            else:
                rows.append(f'{time}  4100  4121 {"ID"[i % 2]} ExampleApp          : synthetic step {i}')
        (root / name).write_text('\n'.join(rows) + '\n', encoding='utf-8')
        counts.append(nav or gps)
    return counts


def track_places(root: Path):
    """track-places.log: 600 Android threadtime rows. From row 3, every
    third row is a GpsSample position whose values have no names
    (`LocationReport(simulated <time> <lat> <lon> <accuracy>)`, degrees).
    Returns how many there are."""
    import math
    rows, count = [], 0
    for i in range(600):
        line = i + 1
        time = f'10-26 15:{i // 600:02d}:{(i // 10) % 60:02d}.{(i % 10) * 100:03d}'
        if line >= 3 and line % 3 == 0:
            a = count / 40 * math.pi
            rows.append(f'{time}  4200  4230 I GpsSample           : LocationReport(simulated {1761490800000 + i * 100} '
                        f'{52.53 + 0.004 * math.cos(a):.6f} {13.38 + 0.006 * math.sin(a):.6f} 5.000000)')
            count += 1
        else:
            rows.append(f'{time}  4100  4121 {"ID"[i % 2]} ExampleApp          : synthetic step {i}')
    (root / 'track-places.log').write_text('\n'.join(rows) + '\n', encoding='utf-8')
    return count


def tile_png(x, y):
    """A synthetic 256 px map tile: a light checkerboard square with a
    border, so tiles show where they meet. No real map data."""
    import zlib
    base = (236, 240, 232) if (x + y) % 2 else (226, 232, 222)
    line = bytearray()
    for row in range(256):
        line.append(0)
        for col in range(256):
            edge = row in (0, 255) or col in (0, 255)
            line += bytes((205, 212, 200) if edge else base)

    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data) & 0xffffffff)
    header = struct.pack('>IIBBBBB', 256, 256, 8, 2, 0, 0, 0)
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', header) + chunk(b'IDAT', zlib.compress(bytes(line))) + chunk(b'IEND', b'')
