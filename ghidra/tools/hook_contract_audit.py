#!/usr/bin/env python3
"""Sweep every `symbols.toml` hook site for hidden register-contract bugs.

`abi_audit.py` validates stack cleanup and ECX/EDX argument registers, but a
hook site can carry a contract no named ABI expresses and still pass both it
and the DIFF harness: the CRT `_CIpow` core took a live x87 `ST(0)` (left by
its thunk's non-popping `FST`) plus a classification dword in EAX — a cdecl
adapter there read its args and returned its value correctly while leaking
one x87 register per call, jamming the 8-slot stack and turning every
subsequent x87 op into the indefinite `QNaN`. This tool flags the signatures
of that class:

  1. x87-consume-first  -- the first x87 instruction reached from the entry
     (with no interposed `call`, whose return may occupy `ST(0)`) consumes ST
     state before anything is loaded => the site expects a live incoming x87
     stack, expressible only via an `x87st0`/`x87pow`-style naked shim.
  2. eax-read-first     -- EAX is read before it is written (again with no
     interposed `call`) while the declared ABI passes nothing in EAX => a
     hidden register argument.
  3. branch-into-patch  -- some instruction in the image branches into
     (VA, VA+5), i.e. INTO the 5-byte detour => corrupted execution when
     taken. Linear-sweep disassembly can desync on data-in-code, so confirm
     any hit against Ghidra xrefs before acting on it.
  4. thunk-fed          -- exactly one direct caller, and that caller performs
     x87 stores right before the call. Alone this is usually just
     floats-passed-by-stack; it matters in CONJUNCTION with signal 1 (the
     `_CIpow` shape).

Entries with `abi = "x87st0"`/`"x87pow"` are register-contract-aware by
construction and are listed annotated, not flagged.

Usage:  hook_contract_audit.py [path-to-host.exe]   (or WOW_EXE env var)
Requires: capstone  (pip install capstone)
"""

import os
import struct
import sys
from pathlib import Path

try:
    import capstone
except ImportError:
    sys.exit("hook_contract_audit: capstone not installed -- run `pip install capstone`")

sys.path.insert(0, str(Path(__file__).resolve().parent))
from abi_audit import image_sections, parse_symbols, rva_to_offset

SYMBOLS_TOML = Path(__file__).resolve().parents[2] / "windows" / "turbo" / "symbols.toml"

# x87 mnemonics that push fresh values without consuming ST state.
X87_PRODUCERS = {
    "fld", "fild", "fld1", "fldz", "fldpi", "fldl2e", "fldl2t", "fldlg2",
    "fldln2", "fbld",
}
# x87 control/status ops that neither read nor write ST data registers.
X87_NEUTRAL = {
    "fnstcw", "fstcw", "fldcw", "fnstsw", "fstsw", "fnclex", "fclex",
    "fninit", "finit", "fnstenv", "fstenv", "fldenv", "fnsave", "fsave",
    "frstor", "wait", "fwait", "ffree", "fincstp", "fdecstp", "fnop", "emms",
}

TERMINATORS = {"ret", "retn", "jmp", "int3", "ljmp", "nop"}

EAX = {"eax", "ax", "al", "ah"}


def exec_ranges(data, base):
    """Executable section ranges as (va_start, size, file_offset) triples.

    Selected by section characteristics (IMAGE_SCN_MEM_EXECUTE = 0x20000000).
    """
    e = struct.unpack_from("<I", data, 0x3C)[0]
    nsec = struct.unpack_from("<H", data, e + 6)[0]
    opt = struct.unpack_from("<H", data, e + 20)[0]
    out = []
    for i in range(nsec):
        o = e + 24 + opt + i * 40
        vsz, va, raw, praw = struct.unpack_from("<IIII", data, o + 8)
        chars = struct.unpack_from("<I", data, o + 36)[0]
        if chars & 0x20000000:
            out.append((base + va, min(vsz, raw) if raw else vsz, praw))
    return out


def sweep_branches(md, data, ranges):
    """Collect every direct call/jmp/jcc target across the executable sections.

    Returns `target_va -> [(kind, from_va)]`. Linear sweep with byte-resync on
    undecodable bytes; treat hits as leads to confirm in Ghidra, not verdicts.
    """
    targets = {}
    md.detail = False
    for va0, size, off in ranges:
        code = data[off : off + size]
        pos = 0
        while pos < size:
            found = False
            for ins in md.disasm(code[pos:], va0 + pos):
                found = True
                m = ins.mnemonic
                if (m == "call" or m == "jmp" or m.startswith("j")) and ins.op_str.startswith("0x"):
                    try:
                        tgt = int(ins.op_str, 16)
                    except ValueError:
                        tgt = None
                    if tgt is not None:
                        kind = "call" if m == "call" else ("jmp" if m == "jmp" else "jcc")
                        targets.setdefault(tgt, []).append((kind, ins.address))
                pos = ins.address + ins.size - va0
            if not found:
                pos += 1  # undecodable byte (data-in-code); resync
    md.detail = True
    return targets


def first_x87(md, data, secs, base, va, limit=200):
    """Classify the first x87 data op on the linear path from `va`.

    Returns ('consume'|'produce'|None, mnemonic, insn_va). Stops at ret/jmp
    and at the first `call` — a callee may return a float in `ST(0)`, so
    anything x87 after it proves nothing about the ENTRY contract.
    """
    off = rva_to_offset(secs, va - base)
    if off is None:
        return None, None, None
    code = bytes(memoryview(data)[off : off + 0x1000])
    n = 0
    for ins in md.disasm(code, va):
        m = ins.mnemonic
        if m.startswith("f") and m not in X87_NEUTRAL:
            if m in X87_PRODUCERS:
                return "produce", m, ins.address
            return "consume", m, ins.address
        if m.startswith("ret") or m in ("jmp", "call"):
            break
        n += 1
        if n > limit:
            break
    return None, None, None


def eax_read_first(md, data, secs, base, va, limit=60):
    """True if EAX is read before written on the linear path from `va`.

    Stops at ret/jmp and at the first `call` (its return value lands in EAX,
    so later reads prove nothing about the entry contract).
    """
    off = rva_to_offset(secs, va - base)
    if off is None:
        return False
    code = bytes(memoryview(data)[off : off + 0x400])
    n = 0
    for ins in md.disasm(code, va):
        rd, wr = ins.regs_access()
        rdn = {ins.reg_name(x) for x in rd}
        wrn = {ins.reg_name(x) for x in wr}
        if rdn & EAX:
            return True
        if wrn & EAX:
            return False
        if ins.mnemonic.startswith("ret") or ins.mnemonic in ("jmp", "call"):
            return False
        n += 1
        if n > limit:
            return False
    return False


def caller_is_x87_thunk(md, data, secs, base, from_va):
    """True if the bytes just before the callsite contain x87 store/exchange ops.

    The `_CIpow` thunk shape: spill ST args right before the call. Floats
    passed by value on the stack look the same, so treat as a weak signal.
    """
    off = rva_to_offset(secs, from_va - base - 24)
    if off is None:
        return False
    code = bytes(memoryview(data)[off : off + 24])
    return any(ins.mnemonic in {"fstp", "fst", "fxch"} for ins in md.disasm(code, from_va - 24))


def main():
    exe = sys.argv[1] if len(sys.argv) > 1 else os.environ.get("WOW_EXE", "")
    if not exe:
        sys.exit("usage: hook_contract_audit.py <WoW.exe>  (or WOW_EXE env)")
    data = Path(exe).read_bytes()
    base, secs = image_sections(data)
    ranges = exec_ranges(data, base)
    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_32)
    md.detail = True

    hooks = list(parse_symbols(SYMBOLS_TOML.read_text()))
    print(f"image base {base:#x}; {len(hooks)} hooks; exec ranges: "
          + ", ".join(f"{va:#x}+{sz:#x}" for va, sz, _ in ranges))

    print("sweeping branches over executable sections ...", flush=True)
    targets = sweep_branches(md, data, ranges)

    listed = strong = 0
    for name, rva, abi, ret, args, preserve in hooks:
        va = base + rva
        notes = []
        aware = abi in ("x87st0", "x87pow")

        kind, mnem, at = first_x87(md, data, secs, base, va)
        if kind == "consume" and not aware:
            notes.append(f"x87-consume-first ({mnem} @ {at:#x})")

        if eax_read_first(md, data, secs, base, va) and abi in ("cdecl", "stdcall"):
            notes.append("eax-read-first")

        refs = targets.get(va, [])
        ncall = sum(1 for k, _ in refs if k == "call")
        njmp = len(refs) - ncall

        interior = [
            (t, k, f)
            for t in range(va + 1, va + 5)
            for k, f in targets.get(t, [])
        ]
        if interior:
            notes.append(
                "BRANCH-INTO-PATCH (confirm in Ghidra): "
                + ", ".join(f"{k}@{f:#x}->{t:#x}" for t, k, f in interior)
            )

        if ncall == 1 and njmp == 0:
            from_va = next(f for k, f in refs if k == "call")
            if caller_is_x87_thunk(md, data, secs, base, from_va):
                notes.append(f"thunk-fed (single x87-spilling caller @ {from_va:#x})")

        if notes or aware:
            listed += 1
            hard = [n for n in notes if not n.startswith("thunk-fed")]
            strong += bool(hard)
            tag = " [register-contract-aware abi]" if aware else ""
            print(f"\n{name}  va={va:#x} abi={abi} calls={ncall} jmps={njmp}{tag}")
            for note in notes:
                print(f"    !! {note}")

    print(f"\n{listed} entries listed (of {len(hooks)}); {strong} with non-thunk-fed signals.")
    sys.exit(1 if strong else 0)


if __name__ == "__main__":
    main()
