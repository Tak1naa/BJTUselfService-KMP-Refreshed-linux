#!/usr/bin/env python3
import argparse,json
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('kotlin',type=Path);p.add_argument('rust',type=Path);a=p.parse_args()
def read(path):
    return json.loads(next(line for line in reversed(path.read_text().splitlines())if line.startswith('{')))
k,r=read(a.kotlin),read(a.rust)
assert k==r,'Kotlin/Rust parser or deterministic SM2 comparison differs'
print('PASS: courses, grades, exams, profile, card, rooms, calendar, course types and fixed-scalar SM2 match Kotlin field by field.')
