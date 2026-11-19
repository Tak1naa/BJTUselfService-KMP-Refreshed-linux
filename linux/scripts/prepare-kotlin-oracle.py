#!/usr/bin/env python3
"""Copy only the original parsers/models into an isolated JVM comparison project."""
from pathlib import Path
import re
root=Path(__file__).resolve().parents[1]
source=root.parent/'multiplatform/shared/src/commonMain/kotlin/team/bjtuss/bjtuselfservice/shared'
dest=root/'tests/kotlin-oracle/build/reference'
files=['data/classroomoccupancy/ClassroomOccupancyHtmlParser.kt','domain/classroomoccupancy/ClassroomOccupancy.kt','domain/course/Course.kt','domain/grade/Grade.kt','domain/grade/GradeRules.kt',
       'data/homework/StrictJson.kt','data/course/CourseScheduleHtmlParser.kt',
       'data/grade/GradeHtmlParser.kt','data/grade/TrainingProgramHtmlParser.kt',
       'auth/LoginHtmlParser.kt','auth/LoginModels.kt','auth/SsoSm2.kt',
       'data/exam/ExamScheduleHtmlParser.kt','domain/home/HomeStatus.kt','data/home/HomeStatusJsonParser.kt']
for name in files:
    data=(source/name).read_text()
    if name=='auth/SsoSm2.kt':
        data=data.replace('internal expect fun secureRandomBytes(size: Int): ByteArray',
             'internal fun secureRandomBytes(size: Int): ByteArray = ByteArray(size).also { java.security.SecureRandom().nextBytes(it) }')
    path=dest/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_text(data)
data=(source/'domain/exam/ExamSchedule.kt').read_text()
model=re.search(r'data class ExamSchedule\([\s\S]*?\n\)',data).group()
path=dest/'domain/exam/ExamSchedule.kt';path.parent.mkdir(parents=True,exist_ok=True)
path.write_text('package team.bjtuss.bjtuselfservice.shared.domain.exam\n'+model+'\n')
print('Prepared original Kotlin parsers and isolated JVM platform random provider.')
