#!/usr/bin/env python3
# DEXAI Debug 内置终端 PTY 宿主：
# 用 pty.fork 创建伪终端并在子进程里把 erase 字符显式设为 DEL(0x7f)，
# 与 xterm 的 Backspace 键（发送 \x7f）保持一致，避免删除键光标乱走。
# 用法：python3 pty-shell.py <shell 路径> [参数...]
import os
import pty
import select
import sys
import termios
import tty


def main() -> int:
    if len(sys.argv) < 2:
        return 2
    shell = sys.argv[1:]
    try:
        pid, fd = pty.fork()
    except OSError:
        return 1

    if pid == 0:
        # 子进程：设置 slave 侧 erase = DEL，再 exec shell
        try:
            attrs = termios.tcgetattr(0)
            attrs[6][termios.VERASE] = 127  # DEL
            termios.tcsetattr(0, termios.TCSANOW, attrs)
        except Exception:
            pass
        try:
            os.execvp(shell[0], shell)
        except Exception:
            os._exit(127)

    # 父进程：主侧转 raw，并双向拷贝 stdin/stdout <-> pty（等价 pty.spawn）
    try:
        tty.setraw(fd)
    except Exception:
        pass
    try:
        while True:
            readable, _, _ = select.select([fd, 0], [], [])
            for source in readable:
                try:
                    data = os.read(source, 4096)
                except OSError:
                    data = b""
                if not data:
                    if source == fd:
                        return 0
                    continue
                target = 1 if source == fd else fd
                try:
                    os.write(target, data)
                except OSError:
                    return 0
    except (KeyboardInterrupt, OSError):
        pass
    finally:
        try:
            os.close(fd)
        except Exception:
            pass
        try:
            os.waitpid(pid, 0)
        except Exception:
            pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
