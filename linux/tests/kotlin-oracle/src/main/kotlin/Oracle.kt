import java.io.File
import team.bjtuss.bjtuselfservice.shared.data.classroomoccupancy.*
import kotlin.io.encoding.Base64
import kotlinx.serialization.json.*
import team.bjtuss.bjtuselfservice.shared.auth.*
import team.bjtuss.bjtuselfservice.shared.data.course.*
import team.bjtuss.bjtuselfservice.shared.data.grade.*
import team.bjtuss.bjtuselfservice.shared.data.exam.*
import team.bjtuss.bjtuselfservice.shared.data.home.*
import team.bjtuss.bjtuselfservice.shared.domain.course.*

fun main(args:Array<String>){
 val root=File(args.single());fun page(s:String)=File(root,s).readText()
 val teachers=(parseTeacherTable(page("teachers.html")) as TeacherTableParseResult.Success).teachersByCourse
 val courses=(parseCourseScheduleTable(page("courses.html"),false,teachers) as CourseScheduleTableParseResult.Success).courses+
    (parseCourseScheduleTable(page("selection.html"),true,teachers) as CourseScheduleTableParseResult.Success).courses
 val grades=(parseGradeTable(page("grades.html")) as GradeTableParseResult.Success).grades
 val exams=(parseExamScheduleTable(page("exams.html")) as ExamScheduleParseResult.Success).exams
 val profile=(parseMisStudentProfile(page("profile.html"),"fixture-user") as ParseResult.Success).value
 val card=(parseHomeStatusJson(page("card.json")) as HomeStatusParseResult.Success).status
 val key=Base64.decode("BCNfHMe8JNA+F0xYeDk+8jpQX/u7gIRDbmOvxyzq5cUyaEGp88qjQT78wi2lAzmdPM6OVMEwDiqrRWqeeO+789s=")
 val point=Sm2Curve.affine(Sm2Field.fromBytes(key.copyOfRange(1,33)),Sm2Field.fromBytes(key.copyOfRange(33,65)))
 val cipher=SsoSm2.encrypt("fixture-Password_123!".encodeToByteArray(),point){U256.fromHex("01".repeat(32))}
 println(buildJsonObject{
  put("courses",buildJsonArray{courses.forEach{c->add(buildJsonObject{put("course_code",c.courseId);put("name",c.courseName);put("teacher",c.courseTeacher);put("location_index",c.courseLocationIndex);put("weeks_raw",c.courseTime);put("place",c.coursePlace);put("is_selection",c.isCurrentSemester)})}})
  put("grades",buildJsonArray{grades.forEach{g->add(buildJsonObject{put("course_name",g.courseName);put("teacher",g.courseTeacher);put("term",g.semester);put("score",g.courseScore);put("credits_raw",g.courseCredits);put("detail",g.detail)})}})
  put("exams",buildJsonArray{exams.forEach{e->add(buildJsonObject{put("exam_type",e.examType);put("course_name",e.courseName);put("time_place_raw",e.examTimeAndPlace);put("status",e.examStatus);put("detail",e.detail)})}})
  put("profile",buildJsonObject{put("student_id",profile.studentId);put("name",profile.name);put("identity",profile.identity);put("department",profile.department)})
  put("card",buildJsonObject{put("card_balance",card.campusCardBalance);put("net_fee",card.networkBalance);put("new_mail",card.newMailCount)})
  put("rooms",buildJsonArray{(parseClassroomOccupancyTable(page("rooms.html")) as ClassroomOccupancyParseResult.Success).rooms.forEach{r->add(buildJsonObject{put("room",r.room);put("capacity",r.capacity);put("cells",buildJsonArray{for(day in 1..7)for(period in 1..7){val n=r.cells[day to period]?.ordinal?:6;add(if(n==6)255 else n)}})})}})
  put("calendar",buildJsonArray{parseAcademicWeeks(page("calendar.html")).forEach{(term,weeks)->weeks.forEach{w->add(buildJsonObject{put("term",term);put("week",w.week);put("monday",w.startDate.toString())})}}})
  put("types",buildJsonArray{(parseProgramCourseTypes(page("program.html")) as TrainingProgramParseResult.Success).courseTypes.toSortedMap().forEach{(code,kind)->add(buildJsonObject{put("course_code",code);put("course_type",mapOf("必修" to "required","限选" to "limited","任选" to "elective","体育" to "pe").getValue(kind))})}})
  put("sm2_fixed",Base64.encode(cipher))
 }.toString())
}
