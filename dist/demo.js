// Fully synthetic data. Never include real user logs in this file.
export function makeDemo() {
  const rows = ['--------- beginning of main'];
  const tags = ['ActivityManager','RenderThread','ConnectivityService','ExampleApp','WifiMonitor.DnsThread'];
  const levels = ['I','D','I','W','D','I','E','V'];
  const messages = ['Starting activity: com.example.demo/.MainActivity','Frame rendered in 12 ms','Network connected: WIFI','Slow response: 420 ms','TCP probeState: 0, connectDelay: 205','화면 준비 완료 • synthetic demo','Request failed: timeout after 3000 ms','Cache lookup complete'];
  for(let i=0;i<480;i++) {
    const seconds = String(Math.floor(i/20)%60).padStart(2,'0');
    const millis = String((i%20)*50).padStart(3,'0');
    const pid = i%3===0 ? '2983' : '12540';
    const tid = i%3===0 ? '4241' : '12610';
    rows.push(`06-18 19:24:${seconds}.${millis}  ${pid} ${tid} ${levels[i%levels.length]} ${tags[i%tags.length].padEnd(22)}: ${messages[i%messages.length]}`);
  }
  return rows.join('\n');
}
