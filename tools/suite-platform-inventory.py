#!/usr/bin/env python3
import argparse
import json
from pathlib import Path, PurePosixPath
import re
import subprocess


def tokens(source):
    result, at = [], 0
    literal = re.compile(r'''(?:[bc])?"(?:\\.|[^"\\])*"|(?:b)?'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|.)|[^'\\\n])'|[^\W\d]\w*|[^\s]''', re.S)
    while at < len(source):
        if source[at].isspace():
            at += 1
            continue
        if source.startswith("//", at):
            end = source.find("\n", at)
            at = len(source) if end < 0 else end + 1
            continue
        if source.startswith("/*", at):
            depth, at = 1, at + 2
            while depth and at < len(source):
                if source.startswith("/*", at):
                    depth, at = depth + 1, at + 2
                elif source.startswith("*/", at):
                    depth, at = depth - 1, at + 2
                else:
                    at += 1
            if depth:
                raise ValueError("unterminated block comment")
            continue
        raw = re.match(r'(?:br|cr|r)(#*)"', source[at:])
        if raw:
            end = source.find('"' + raw[1], at + len(raw[0]))
            if end < 0:
                raise ValueError("unterminated raw string")
            end += 1 + len(raw[1])
            result.append(source[at:end])
            at = end
            continue
        match = literal.match(source, at)
        if not match:
            raise ValueError("unrecognized Rust token")
        if match[0] == '"':
            raise ValueError("unterminated string")
        result.append(match[0])
        at = match.end()
    return result


def cfg(expression, macos):
    text = "".join(expression)
    if text in ("test", "debug_assertions", "unix"):
        return True
    if text == 'target_os="macos"':
        return macos
    if text == 'target_os="linux"':
        return not macos
    if text == "windows":
        return False
    if expression and expression[0] in ("all", "any", "not") and expression[1] == "(":
        arguments, start, depth = [], 2, 0
        for index in range(2, len(expression) - 1):
            depth += (expression[index] == "(") - (expression[index] == ")")
            if expression[index] == "," and depth == 0:
                arguments.append(expression[start:index])
                start = index + 1
        if start < len(expression) - 1:
            arguments.append(expression[start:-1])
        values = [cfg(argument, macos) for argument in arguments]
        if expression[0] == "not":
            if len(values) != 1:
                raise ValueError("cfg not requires one argument")
            return not values[0]
        return all(values) if expression[0] == "all" else any(values)
    raise ValueError("unsupported platform cfg: " + text)


def source_inventory(path, source, inherited=None):
    values = tokens(source)
    pairs, stack, attributes = {}, [], []
    for index, value in enumerate(values):
        if value in ("(", "[", "{"):
            stack.append(index)
        elif value in (")", "]", "}"):
            if not stack or values[stack[-1]] != {")": "(", "]": "[", "}": "{"}[value]:
                raise ValueError("unbalanced Rust delimiters")
            pairs[stack.pop()] = index
    if stack:
        raise ValueError("unbalanced Rust delimiters")
    for index, value in enumerate(values[:-2]):
        if value != "#":
            continue
        inner = values[index + 1] == "!"
        opening = index + 2 if inner else index + 1
        if values[opening] != "[":
            continue
        end, target = pairs[opening], pairs[opening] + 1
        while target + 1 < len(values) and values[target] == "#":
            target = pairs[target + (2 if values[target + 1] == "!" else 1)] + 1
        attributes.append((index, values[opening + 1:end], target, inner))
    spans = [(0, len(values), *inherited)] if inherited else []
    for index, attribute, target, inner in attributes:
        if attribute[0] == "cfg_attr" and ('"macos"' in attribute or "ignore" in attribute):
            raise ValueError("unsupported platform or ignore cfg_attr")
        if attribute[:2] != ["cfg", "("] or '"macos"' not in attribute:
            continue
        linux, macos = cfg(attribute[2:-1], False), cfg(attribute[2:-1], True)
        if inner:
            ancestors = [(begin, end) for begin, end in pairs.items() if values[begin] == "{" and begin < index < end]
            begin, end = max(ancestors, default=(0, len(values)))
        else:
            end = target
            while end < len(values) and values[end] not in ("{", ";", ",", "}"):
                end = pairs[end] + 1 if values[end] in ("(", "[") else end + 1
            if end == len(values):
                raise ValueError("platform cfg has no following item")
            begin, end = target, pairs.get(end, end)
        spans.append((begin, end, linux, macos))
    modules, external = [], []
    def state(position):
        return tuple(all(span[column] for span in spans if span[0] <= position < span[1]) for column in (2, 3))
    for index, value in enumerate(values[:-2]):
        if value == "mod" and values[index + 2] in ("{", ";"):
            if values[index + 2] == "{":
                modules.append((index, pairs[index + 2], values[index + 1]))
            elif state(index) != (True, True):
                if any(body[0] == "path" and destination <= index <= destination + 5 for _, body, destination, _ in attributes):
                    raise ValueError("unsupported #[path] on platform-gated module")
                names = [name for begin, finish, name in modules if begin < index < finish]
                external.append((names + [values[index + 1]], state(index)))
    result = {key: [] for key in ("absent", "partial", "runtime_branches", "ignored")}
    for _, attribute, target, _ in attributes:
        head = attribute[:attribute.index("(")] if "(" in attribute else attribute
        if head[-1:] == ["test"] and head != ["test"]:
            raise ValueError("unsupported test attribute")
        if attribute != ["test"]:
            continue
        if values[target] != "fn":
            raise ValueError("unsupported test item")
        opening = target + 2
        while values[opening] != "{":
            opening += 1
        end = pairs[opening]
        name = "::".join([name for begin, finish, name in modules if begin < target < finish] + [values[target + 1]])
        identity = path + "::" + name
        absent = state(target) == (False, True)
        partial = state(target) == (True, True) and any(target < begin < end and state(begin) == (False, True) for begin, _, _, _ in spans)
        if absent:
            result["absent"].append(identity)
        elif partial:
            result["partial"].append(identity)
        if not absent:
            for index in range(opening + 1, end - 2):
                if values[index:index + 3] == ["cfg", "!", "("]:
                    expression = values[index + 3:pairs[index + 2]]
                    if '"macos"' in expression and cfg(expression, False) != cfg(expression, True):
                        result["runtime_branches"].append(identity)
                        break
        if any(body[0] == "ignore" and destination == target for _, body, destination, _ in attributes):
            result["ignored"].append(identity)
    return result, external


def inventory(ref="HEAD"):
    root = Path(__file__).resolve().parents[1]
    paths = subprocess.check_output(["git", "ls-tree", "-r", "--name-only", ref], text=True, cwd=root).splitlines()
    sources, reports, inherited = {}, {}, {}
    for path in paths:
        if not path.startswith("crates/") or not path.endswith(".rs"):
            continue
        sources[path] = subprocess.check_output(["git", "show", f"{ref}:{path}"], text=True, cwd=root)
    def read(path, gate=None):
        source = sources[path]
        if not any(word in source for word in ("macos", "ignore")):
            if gate is None:
                return {}, []
        try:
            report, external = source_inventory(path, source, gate)
        except ValueError as error:
            raise ValueError(f"{path}: {error}") from error
        children = []
        for names, child_gate in external:
            file = PurePosixPath(path)
            base = file.parent if file.stem in ("lib", "main", "mod") or "/tests/" in path else file.with_suffix("")
            module = base.joinpath(*names)
            candidates = [str(module.with_suffix(".rs")), str(module / "mod.rs")]
            found = [candidate for candidate in candidates if candidate in sources]
            if len(found) != 1:
                raise ValueError(f"{path}: cannot resolve gated module {'::'.join(names)}")
            children.append((found[0], child_gate))
        return report, children
    initial = {path: read(path) for path in sources}
    descendants = {child for _, children in initial.values() for child, _ in children}
    pending = [(path, None) for path in sources if path not in descendants]
    while pending:
        path, gate = pending.pop()
        if path in inherited:
            if inherited[path] != gate:
                raise ValueError(f"{path}: conflicting module platform gates")
            continue
        inherited[path] = gate
        reports[path], children = read(path, gate)
        pending.extend(children)
    if descendants - inherited.keys():
        raise ValueError("unreachable platform module graph")
    keys = ("absent", "partial", "runtime_branches", "ignored")
    return {key: sorted({entry for report in reports.values() for entry in report.get(key, [])}) for key in keys}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--ref", default="HEAD")
    print(json.dumps(inventory(parser.parse_args().ref), indent=2, ensure_ascii=False))
