#!/usr/bin/env python3
"""Drive the headless test VM (HEADLESS=1 scripts/qemu-vm.sh) over QMP.

  qemu-drive.py shot out.png            screenshot (saved in the VM folder)
  qemu-drive.py type 'text'             type text (US layout)
  qemu-drive.py key ret | tab | ctrl-alt-f2 | ...   press a key combination (qemu key names joined by -)
  qemu-drive.py hmp 'info status'       any human-monitor command
"""
import json, os, socket, sys, time

SOCK = os.path.expanduser(os.environ.get("SAFORA_VM", "~/.local/share/safora/vm")) + "/qmp.sock"

SHIFTED = {'~': 'grave_accent', '!': '1', '@': '2', '#': '3', '$': '4', '%': '5', '^': '6', '&': '7', '*': '8',
           '(': '9', ')': '0', '_': 'minus', '+': 'equal', '{': 'bracket_left', '}': 'bracket_right', '|': 'backslash',
           ':': 'semicolon', '"': 'apostrophe', '<': 'comma', '>': 'dot', '?': 'slash'}
PLAIN = {' ': 'spc', '-': 'minus', '=': 'equal', '[': 'bracket_left', ']': 'bracket_right', '\\': 'backslash',
         ';': 'semicolon', "'": 'apostrophe', ',': 'comma', '.': 'dot', '/': 'slash', '`': 'grave_accent', '\n': 'ret'}


class QMP:
    def __init__(self):
        self.s = socket.socket(socket.AF_UNIX)
        self.s.connect(SOCK)
        self.f = self.s.makefile("rw")
        self.f.readline()
        self.cmd("qmp_capabilities")

    def cmd(self, name, **args):
        self.f.write(json.dumps({"execute": name, "arguments": args}) + "\n")
        self.f.flush()
        while True:
            r = json.loads(self.f.readline())
            if "return" in r or "error" in r:
                if "error" in r:
                    raise SystemExit(r["error"])
                return r["return"]

    def keys(self, names):
        self.cmd("send-key", keys=[{"type": "qcode", "data": k} for k in names], **{"hold-time": 60})
        time.sleep(0.07)


def char_keys(c):
    if c.isalpha():
        return ["shift", c.lower()] if c.isupper() else [c]
    if c.isdigit():
        return [c]
    if c in SHIFTED:
        return ["shift", SHIFTED[c]]
    return [PLAIN[c]]


def main():
    q = QMP()
    op, arg = sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else ""
    if op == "shot":
        # QEMU runs in the sandbox, where the VM folder is /vm: the file lands in ~/.local/share/safora/vm.
        q.cmd("screendump", filename="/vm/" + os.path.basename(arg), format="png")
    elif op == "type":
        for c in arg:
            q.keys(char_keys(c))
    elif op == "key":
        for combo in arg.split():
            q.keys(combo.split("-"))
    elif op == "hmp":
        print(q.cmd("human-monitor-command", **{"command-line": arg}))


if __name__ == "__main__":
    main()
