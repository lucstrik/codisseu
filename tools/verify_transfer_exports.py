#!/usr/bin/env python3
"""Independently verify the synthetic Rust export corpus; never execute its code.

Usage: verify_transfer_exports.py DIRECTORY [--output REPORT.json]
Only Python's standard library is required. openpyxl is an additional reader when
available; generated Python is parsed with ast and restricted literal inspection.
"""
import argparse
import ast
import csv
import decimal
import hashlib
from html.parser import HTMLParser
import io
import json
from pathlib import Path
import sys
import warnings
import xml.etree.ElementTree as ET
import zipfile


def cell_text(cell):
    if cell == "Null":
        return "null"
    kind, value = next(iter(cell.items()))
    if kind == "Bool":
        return str(value).lower()
    if kind == "Binary":
        return "0x" + bytes(value).hex().upper()
    return value


def json_value(cell):
    if cell == "Null":
        return None
    kind, value = next(iter(cell.items()))
    if kind == "Number":
        return decimal.Decimal(value)
    if kind == "Json":
        return json.loads(value, parse_float=decimal.Decimal, parse_int=decimal.Decimal)
    if kind == "Binary":
        return "0x" + bytes(value).hex()
    return value


class TableParser(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.rows, self.row, self.value = [], [], None

    def handle_starttag(self, tag, attrs):
        assert tag != "script", "source data became an executable script element"
        if tag in ("td", "th"):
            self.value = ""
        elif tag == "br" and self.value is not None:
            self.value += "\n"

    def handle_endtag(self, tag):
        if tag in ("td", "th"):
            self.row.append(self.value)
            self.value = None
        elif tag == "tr":
            self.rows.append(self.row)
            self.row = []

    def handle_data(self, data):
        if self.value is not None:
            self.value += data


def python_literal(node):
    if isinstance(node, ast.Constant):
        return node.value
    if isinstance(node, ast.List):
        return [python_literal(value) for value in node.elts]
    if isinstance(node, ast.Dict):
        return {python_literal(key): python_literal(value) for key, value in zip(node.keys, node.values)}
    if isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and node.func.id == "Decimal":
        assert len(node.args) == 1 and not node.keywords
        return decimal.Decimal(python_literal(node.args[0]))
    if (isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)
            and isinstance(node.func.value, ast.Name) and node.func.value.id == "bytes"
            and node.func.attr == "fromhex"):
        assert len(node.args) == 1 and not node.keywords
        return bytes.fromhex(python_literal(node.args[0]))
    raise AssertionError(f"Unexpected generated Python expression: {ast.dump(node)}")


def verify_python(text, fixture):
    tree = ast.parse(text, filename="generated-dataframe.py")
    assignments = [node for node in tree.body if isinstance(node, ast.Assign)
                   and any(isinstance(target, ast.Name) and target.id == "data" for target in node.targets)]
    assert len(assignments) == 1
    data = python_literal(assignments[0].value)
    assert list(data) == fixture["columns"]
    for column, name in enumerate(fixture["columns"]):
        expected = []
        for row in fixture["rows"]:
            cell = row[column]
            if cell == "Null":
                expected.append(None)
            else:
                kind, value = next(iter(cell.items()))
                expected.append(decimal.Decimal(value) if kind == "Number" else bytes(value) if kind == "Binary" else value)
        assert data[name] == expected, (name, data[name], expected)


def verify_xlsx(path, fixture, query):
    ns = {"s": "http://schemas.openxmlformats.org/spreadsheetml/2006/main"}
    assert path.read_bytes()[:4] == b"PK\x03\x04"
    with zipfile.ZipFile(path) as archive:
        assert archive.testzip() is None, "ZIP CRC mismatch"
        names = archive.namelist()
        assert len(names) == len(set(names))
        required = {"[Content_Types].xml", "_rels/.rels", "xl/workbook.xml",
                    "xl/_rels/workbook.xml.rels", "xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"}
        assert required <= set(names)
        for name in names:
            assert ".." not in Path(name).parts and not name.startswith("/")
            ET.fromstring(archive.read(name))
        workbook = ET.fromstring(archive.read("xl/workbook.xml"))
        assert [sheet.attrib["name"] for sheet in workbook.findall("s:sheets/s:sheet", ns)] == ["Result 1", "Query"]
        sheet = ET.fromstring(archive.read("xl/worksheets/sheet1.xml"))
        assert not sheet.findall(".//s:f", ns), "string data must never become formulas"
        cells = {cell.attrib["r"]: cell for cell in sheet.findall(".//s:c", ns)}
        text = lambda cell: "".join(cell.itertext())
        for column, name in enumerate(fixture["columns"]):
            assert text(cells[f"{chr(65 + column)}1"]) == name
        assert text(cells["B2"]) == fixture["rows"][0][1]["Text"]
        assert list(cells["C2"]) == [] and "t" not in cells["C2"].attrib, "NULL must remain a blank cell"
        assert cells["C3"].attrib["t"] == "inlineStr", "empty string must retain string metadata"
        assert cells["D2"].attrib["t"] == "inlineStr"
        assert text(cells["D2"]) == fixture["rows"][0][3]["Number"], "large decimal lost digits"
        assert cells["E2"].attrib["t"] == "inlineStr" and text(cells["E2"]) == "=SUM(A1:A2)"
        assert text(cells["F2"]) == "0x00FF0D0A"
        assert cells["G2"].attrib["t"] == "b" and text(cells["G2"]) == "1"
        query_sheet = ET.fromstring(archive.read("xl/worksheets/sheet2.xml"))
        assert "".join(query_sheet.find(".//s:c", ns).itertext()) == query
    try:
        import openpyxl
    except ImportError:
        return {"zip_crc": "passed", "ooxml": "passed", "openpyxl": "unavailable; not counted"}
    with warnings.catch_warnings(record=True) as captured:
        book = openpyxl.load_workbook(path, data_only=False)
    assert book.sheetnames == ["Result 1", "Query"]
    sheet = book["Result 1"]
    assert sheet["B2"].value == fixture["rows"][0][1]["Text"]
    assert sheet["C2"].value is None
    assert sheet["D2"].value == fixture["rows"][0][3]["Number"]
    assert sheet["E2"].value == "=SUM(A1:A2)" and sheet["E2"].data_type != "f"
    assert sheet["F2"].value == "0x00FF0D0A"
    assert book["Query"]["A1"].value == query
    return {"zip_crc": "passed", "ooxml": "passed", "openpyxl": openpyxl.__version__,
            "reader_warnings": [str(warning.message) for warning in captured]}


def verify(directory):
    manifest = json.loads((directory / "manifest.json").read_text())
    fixture = json.loads((directory / "fixture.json").read_text())
    assert len(manifest["formats"]) == 19
    expected_text = [[cell_text(cell) for cell in row] for row in fixture["rows"]]
    checks, xlsx = [], None
    separators = {"csv": ",", "tsv": "\t", "pipe": "|", "semicolon": ";", "csv-scripted": ","}
    for item in manifest["formats"]:
        path = directory / item["file"]
        assert path.parent == directory and path.is_file()
        payload = path.read_bytes()
        assert len(payload) == item["bytes"] and payload
        id_ = item["id"]
        methods = ["nonempty", "byte length", "SHA256"]
        if id_ == "xlsx":
            xlsx = verify_xlsx(path, fixture, manifest["query"])
            methods += ["ZIP CRC", "OOXML parsing", "typed cells", "formula prevention"]
        else:
            text = payload.decode("utf-8")
            assert "Ação" in text and "123456789012345678901.123456789" in text
            methods.append("Unicode and decimal digits")
            if id_ in separators:
                rows = list(csv.reader(io.StringIO(text, newline=""), delimiter=separators[id_]))
                expected = [row[:] for row in expected_text]
                for row_index, row in enumerate(fixture["rows"]):
                    for column, cell in enumerate(row):
                        if cell == "Null":
                            expected[row_index][column] = "null" if id_ == "csv-scripted" else r"\N"
                assert rows == [fixture["columns"], *expected], (id_, rows, expected)
                methods.append("independent csv.reader with real separator and embedded newline")
            elif id_ == "json":
                rows = json.loads(text, parse_float=decimal.Decimal, parse_int=decimal.Decimal)
                expected = [{name: json_value(cell) for name, cell in zip(fixture["columns"], row)} for row in fixture["rows"]]
                assert rows == expected
                methods.append("JSON parser with Decimal precision")
            elif id_ == "xml":
                root = ET.fromstring(text)
                assert root.tag == "data"
                actual = [[element.text or "" for element in row] for row in root.findall("row")]
                assert actual == expected_text
                methods.append("ElementTree structure and decoded values")
            elif id_ in ("html-groovy", "html-js"):
                parser = TableParser()
                parser.feed(text)
                assert parser.rows == [fixture["columns"], *expected_text], (id_, parser.rows)
                methods.append("HTMLParser decoded values and no script injection")
            elif id_ == "python-dataframe":
                verify_python(text, fixture)
                methods.append("ast.parse and restricted literal/Decimal/bytes inspection; no execution")
            elif id_ == "markdown":
                assert "pipe&#124;" in text and "&lt;script&gt;" in text and "<br/>" in text
                methods.append("pipe and markup escaping")
        checks.append({**item, "sha256": hashlib.sha256(payload).hexdigest(), "status": "passed", "checks": methods})
    return {"status": "passed", "formats_verified": len(checks), "data_rows": len(fixture["rows"]),
            "scope": "Independent readers over synthetic exports; no DataGrip byte-equality claim; generated Python never executed",
            "python": sys.version.split()[0], "xlsx": xlsx, "formats": checks,
            "primary_sources": ["https://docs.python.org/3/library/zipfile.html", "https://docs.python.org/3/library/ast.html",
                                "https://learn.microsoft.com/en-us/office/open-xml/spreadsheet/working-with-formulas"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    result = verify(args.directory.resolve())
    rendered = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.write_text(rendered)
    print(rendered, end="")


if __name__ == "__main__":
    main()
