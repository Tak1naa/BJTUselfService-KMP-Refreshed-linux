#!/usr/bin/env python3
"""Measure warm offline CLI processes with a private, synthetic SQLite database."""
import argparse,json,os,sqlite3,statistics,subprocess,tempfile,time
from datetime import datetime,timedelta,timezone
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('binary',type=Path);p.add_argument('--runs',type=int,default=100);p.add_argument('--max-ms',type=float,default=20);p.add_argument('--max-bytes',type=int,default=10*1024*1024);p.add_argument('--output',type=Path);a=p.parse_args();binary=a.binary.resolve()
if a.runs<20: p.error('at least 20 runs are required for p95')
with tempfile.TemporaryDirectory(prefix='campus-bench-') as d:
    root=Path(d);env=os.environ.copy()
    for k,n in [('HOME','home'),('XDG_DATA_HOME','data'),('XDG_CONFIG_HOME','config'),('XDG_CACHE_HOME','cache'),('XDG_STATE_HOME','state'),('XDG_RUNTIME_DIR','runtime')]:env[k]=str(root/n)
    env['NO_COLOR']='1';env['CAMPUS_PORTAL_ENDPOINTS_MIS']='http://invalid.example/'
    subprocess.run([binary,'cache','info','--json'],env=env,check=True,stdout=subprocess.DEVNULL)
    db=sqlite3.connect(root/'data/campus-portal/campus.db');now=int(time.time())
    db.execute('INSERT INTO account(id,student_id,name,created_at) VALUES (1,?,?,?)',('fixture-user','合成测试',now))
    db.execute('INSERT INTO pref VALUES (?,?,?)',('','default_account','fixture-user'))
    school_date=datetime.now(timezone(timedelta(hours=8))).date()
    monday=school_date-timedelta(days=school_date.weekday())
    course=dict(course_code='C312009B',name='操作系统',teacher='测试教师',location_index=school_date.isoweekday(),weeks_raw='第1-30周',weeks_mask=sum(1<<w for w in range(1,31)),place='SY101',is_selection=False)
    grade=dict(course_name='操作系统',course_code='C312009B',teacher='测试教师',term='2026-2027-1',score_raw='95',score_letter='A',score_value=95,credits_raw='3',credits=3,detail='合成成绩')
    exam=dict(exam_type='期末',course_name='操作系统',time_place_raw='待安排',status='待安排',detail='',starts_at=None,ends_at=None,location=None)
    card=dict(card_balance='18.50',net_fee='4',new_mail='0')
    db.execute('INSERT INTO dataset_row VALUES (?,?,?,?)',(0,'calendar',0,json.dumps(dict(term='2026-2027-1',week=1,monday=monday.isoformat()))))
    for dataset,rows in [('courses',[course]*100),('grades',[grade]*1000),('exams',[exam]*30),('card',[card])]:
        db.executemany('INSERT INTO dataset_row VALUES (?,?,?,?)',[(1,dataset,i,json.dumps(v,ensure_ascii=False))for i,v in enumerate(rows)])
        db.execute('INSERT INTO sync_state(account_id,dataset,last_success_at,last_status) VALUES(?,?,?,?)',(1,dataset,now,'success'))
    db.executemany('INSERT INTO grade(account_id,ordinal,course_name,course_code,teacher,term,score_raw,score_letter,score_value,credits_raw,credits,detail) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)',[(1,i,grade['course_name'],grade['course_code'],grade['teacher'],grade['term'],grade['score_raw'],grade['score_letter'],grade['score_value'],grade['credits_raw'],grade['credits'],grade['detail']) for i in range(1000)])
    db.commit();db.close()
    commands=[['--version'],['schedule','today','--offline','--json'],['grades','--offline','--json'],['exams','--upcoming','--offline','--json'],['card','--offline','--json']]
    result={'binary':str(binary),'bytes':binary.stat().st_size,'runs':a.runs,'cache':'warm synthetic; includes process launch and output to /dev/null','commands':[]}
    for cmd in commands:
        for _ in range(5):subprocess.run([binary,*cmd],env=env,check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        samples=[]
        for _ in range(a.runs):
            start=time.perf_counter_ns();subprocess.run([binary,*cmd],env=env,check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL);samples.append((time.perf_counter_ns()-start)/1e6)
        samples.sort();result['commands'].append({'command':' '.join(cmd),'median_ms':statistics.median(samples),'p95_ms':samples[int((len(samples)-1)*.95)],'max_ms':max(samples)})
    text=json.dumps(result,ensure_ascii=False,indent=2);print(text)
    if a.output:a.output.write_text(text+'\n')
    failed=result['bytes']>a.max_bytes or any(v['p95_ms']>(3 if v['command']=='--version' else a.max_ms) for v in result['commands'])
    raise SystemExit(1 if failed else 0)
