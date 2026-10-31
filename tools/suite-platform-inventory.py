#!/usr/bin/env python3
import argparse
import json
from pathlib import Path, PurePosixPath
import posixpath
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
    if text in ('target_os="macos"', 'target_vendor="apple"'):
        return macos
    if text == 'target_os="linux"':
        return not macos
    if text in ("windows", 'target_os="windows"'):
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


def platform_cfg(expression):
    return any(word in ("windows", "unix") or word.startswith("target_") for word in expression)


def path_literal(value):
    raw = re.fullmatch(r'r(#{0,255})"(.*)"\1', value, re.S)
    if raw:
        result = raw[2].replace("\r\n", "\n")
        if "\r" not in result:
            return result
    elif value.startswith('"') and value.endswith('"'):
        source, result, at = value[1:-1].replace("\r\n", "\n"), [], 0
        escapes = {"n": "\n", "r": "\r", "t": "\t", "0": "\0", "\\": "\\", '"': '"', "'": "'"}
        while at < len(source):
            character, at = source[at], at + 1
            if character == "\r":
                break
            if character != "\\":
                result.append(character)
                continue
            if at == len(source):
                break
            character, at = source[at], at + 1
            if character in escapes:
                result.append(escapes[character])
            elif character == "\n":
                while at < len(source) and source[at] in " \t\n\r":
                    at += 1
            elif character == "x" and re.fullmatch(r"[0-7][0-9a-fA-F]", source[at:at + 2]):
                result.append(chr(int(source[at:at + 2], 16)))
                at += 2
            elif character == "u" and (escape := re.match(r"\{([0-9a-fA-F][0-9a-fA-F_]*)\}", source[at:])):
                digits = escape[1].replace("_", "")
                scalar = int(digits, 16)
                if len(digits) > 6 or scalar > 0x10ffff or 0xd800 <= scalar <= 0xdfff:
                    break
                result.append(chr(scalar))
                at += len(escape[0])
            else:
                break
        else:
            return "".join(result)
    raise ValueError("unsupported #[path] literal")


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
        if target < len(values) and values[target] == "pub":
            target += 1
            if target < len(values) and values[target] == "(":
                target = pairs[target] + 1
        attributes.append((index, values[opening + 1:end], target, inner))
    paths = {}
    for _, attribute, target, inner in attributes:
        if attribute[:1] == ["cfg_attr"] and "path" in attribute:
            raise ValueError("unsupported #[path] attribute")
        if attribute[:1] != ["path"]:
            continue
        if inner or len(attribute) != 3 or attribute[1] != "=" or target in paths or target + 2 >= len(values) or values[target] != "mod" or values[target + 2] not in ("{", ";"):
            raise ValueError("unsupported #[path] attribute")
        paths[target] = path_literal(attribute[2])
    spans = [(0, len(values), *inherited)] if inherited else []
    for index, attribute, target, inner in attributes:
        if attribute[0] == "cfg_attr" and (platform_cfg(attribute) or "ignore" in attribute):
            raise ValueError("unsupported platform or ignore cfg_attr")
        if attribute[:2] != ["cfg", "("] or not platform_cfg(attribute):
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
                modules.append((index, pairs[index + 2], values[index + 1], paths.get(index)))
            else:
                route = [(name, override) for begin, finish, name, override in modules if begin < index < finish]
                external.append((route + [(values[index + 1], paths.get(index))], state(index)))
    result = {key: [] for key in ("absent", "partial", "runtime_branches", "ignored", "linux_present")}
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
        name = "::".join([name for begin, finish, name, _ in modules if begin < target < finish] + [values[target + 1]])
        identity = path + "::" + name
        if state(target)[0]:
            result["linux_present"].append(identity)
        absent = state(target) == (False, True)
        partial = state(target) == (True, True) and any(target < begin < end and state(begin) == (False, True) for begin, _, _, _ in spans)
        if absent:
            result["absent"].append(identity)
        elif partial:
            result["partial"].append(identity)
        if state(target) == (True, True):
            for index in range(opening + 1, end - 2):
                if values[index:index + 3] == ["cfg", "!", "("]:
                    expression = values[index + 3:pairs[index + 2]]
                    if platform_cfg(expression) and cfg(expression, False) != cfg(expression, True):
                        result["runtime_branches"].append(identity)
                        break
        if any(body[0] == "ignore" and destination == target for _, body, destination, _ in attributes):
            result["ignored"].append(identity)
    return result, external


def inventory(ref="HEAD"):
    root = Path(__file__).resolve().parents[1]
    paths = subprocess.check_output(["git", "ls-tree", "-r", "--name-only", ref], text=True, cwd=root, stderr=subprocess.DEVNULL).splitlines()
    sources, reports, inherited = {}, {}, {}
    for path in paths:
        if not path.startswith("crates/") or not path.endswith(".rs"):
            continue
        sources[path] = subprocess.check_output(["git", "show", f"{ref}:{path}"], text=True, cwd=root, stderr=subprocess.DEVNULL)
    def root_context(path):
        file = PurePosixPath(path)
        return path, str(file.parent if file.stem in ("lib", "main", "mod") or file.parent.name == "tests" else file.with_suffix(""))
    def read(context, gate=None, discover=False):
        path, directory = context
        source = sources[path]
        try:
            report, external = source_inventory(path, source, gate)
        except ValueError as error:
            raise ValueError(f"{path}: {error}") from error
        children = []
        for route, child_gate in external:
            file = PurePosixPath(path)
            module = PurePosixPath(directory)
            for index, (name, override) in enumerate(route):
                if override is None:
                    module /= name
                else:
                    module = (file.parent if index == 0 else module) / override
            module = PurePosixPath(posixpath.normpath(str(module)))
            explicit = route[-1][1] is not None
            candidates = [str(module)] if explicit else [str(module) + ".rs", str(module / "mod.rs")]
            found = [candidate for candidate in candidates if candidate in sources]
            if discover:
                children.extend(((child, str(PurePosixPath(child).parent) if explicit else str(module)), child_gate) for child in found)
                continue
            if not found and child_gate == (True, True) and all(override is None for _, override in route):
                continue
            if len(found) != 1:
                raise ValueError(f"{path}: cannot resolve gated module {'::'.join(name for name, _ in route)}")
            children.append(((found[0], str(PurePosixPath(found[0]).parent) if explicit else str(module)), child_gate))
        return report, children
    graph, pending = {}, [root_context(path) for path in sources]
    while pending:
        context = pending.pop()
        if context in graph:
            continue
        _, children = read(context, discover=True)
        graph[context] = [child for child, _ in children]
        pending.extend(graph[context])
    descendants = {child[0] for children in graph.values() for child in children}
    roots = {root_context(path) for path in sources if path not in descendants or PurePosixPath(path).parent.name == "tests"}
    reached, pending = set(), list(roots)
    while True:
        while pending:
            context = pending.pop()
            if context not in reached:
                reached.add(context)
                pending.extend(graph[context])
        remaining = sources.keys() - {path for path, _ in reached}
        if not remaining:
            break
        descendants = {child[0] for context, children in graph.items() if context[0] in remaining for child in children}
        extra = {root_context(path) for path in remaining - descendants}
        if not extra:
            raise ValueError("unreachable platform module graph")
        roots.update(extra)
        pending.extend(extra)
    pending = [(context, (True, True)) for context in roots]
    while pending:
        context, gate = pending.pop()
        previous = inherited.get(context, (False, False))
        combined = tuple(before or after for before, after in zip(previous, gate))
        if context in inherited and previous == combined:
            continue
        inherited[context] = combined
        reports[context], children = read(context, combined)
        pending.extend(children)
    keys = ("absent", "partial", "runtime_branches", "ignored")
    result = {key: {entry for report in reports.values() for entry in report.get(key, [])} for key in keys}
    result["absent"] -= {entry for report in reports.values() for entry in report.get("linux_present", [])}
    return {key: sorted(entries) for key, entries in result.items()}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--ref", default="HEAD")
    print(json.dumps(inventory(parser.parse_args().ref), indent=2, ensure_ascii=False))
