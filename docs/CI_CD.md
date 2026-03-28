[Back to README](../README.md)

# CI/CD Integration

Use `send_and_expect` to gate deployments on hardware response. Semantic exit
codes are always active — no flag required. Below is a complete GitHub Actions
workflow for hardware-in-the-loop testing.

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | SUCCESS — pattern matched |
| 1 | PATTERN_NOT_MATCHED |
| 2 | CONNECTION_ERROR |
| 3 | TIMEOUT |
| 4 | INVALID_INPUT |
| 5 | INTERNAL_ERROR |

Errors are written as structured JSON to stderr: `{"error":"...","message":"...","exit_code":N}`

## GitHub Actions Example

```yaml
# .github/workflows/hardware-test.yml
name: Hardware-in-the-loop test
on: [push]

jobs:
  serial-test:
    runs-on: [self-hosted, has-serial]
    steps:
      - uses: actions/checkout@v4

      - name: Install serialink
        run: cargo install serialink

      - name: Flash firmware
        run: esptool.py --port /dev/ttyUSB0 write_flash 0x0 firmware.bin

      - name: Wait for boot message
        run: >
          serialink send /dev/ttyUSB0 "" -e "system initialized" -t 30

      - name: Run AT command smoke test
        run: >
          serialink send /dev/ttyUSB0 "AT\r\n" -e "^OK" -t 5

      - name: Capture boot log on failure
        if: failure()
        run: serialink monitor /dev/ttyUSB0 --duration 5 > boot-log.json

      - name: Upload log artifact
        if: failure()
        uses: actions/upload-artifact@v4
        with:
          name: serial-boot-log
          path: boot-log.json
```

## Key Patterns

- **Gate on hardware response**: `serialink send ... -e "pattern"` returns exit code 0 on match, 1 on no match, 3 on timeout. No extra flag needed.
- **Capture diagnostics on failure**: Use `if: failure()` to grab serial logs when a step fails. Output is JSON by default.
- **Self-hosted runners**: Hardware-in-the-loop tests require `[self-hosted, has-serial]` runners with physical serial devices attached.

---

## CI/CD 集成（中文）

在 CI/CD 场景中，serialink 的核心价值是把硬件串口验证变成可脚本化、可失
败即退出的流水线步骤。`send_and_expect` 直接作为门禁条件，语义退出码（0–5）
始终生效，无需额外参数；失败时再补抓 `monitor` 日志，适合把串口测试接入真
实自动化流程。输出默认为 JSON，可用 `--human` 切换为人类可读格式。
