//! Deterministic RNG exists only in this offline comparison example, never auth.
use base64::{Engine, engine::general_purpose::STANDARD};
use campus_core::parse;
use serde_json::json;
use sm2::elliptic_curve::rand_core::{TryCryptoRng, TryRng};
struct Fixed;
impl TryRng for Fixed {
    type Error = std::convert::Infallible;
    fn try_next_u32(&mut self) -> std::result::Result<u32, Self::Error> {
        Ok(0x01010101)
    }
    fn try_next_u64(&mut self) -> std::result::Result<u64, Self::Error> {
        Ok(0x0101010101010101)
    }
    fn try_fill_bytes(&mut self, b: &mut [u8]) -> std::result::Result<(), Self::Error> {
        b.fill(1);
        Ok(())
    }
}
impl TryCryptoRng for Fixed {}
fn main() {
    let root = std::path::PathBuf::from(std::env::args().nth(1).unwrap());
    let page = |s: &str| std::fs::read_to_string(root.join(s)).unwrap();
    let teachers = parse::teachers(&page("teachers.html")).unwrap();
    let mut courses = parse::courses(&page("courses.html"), false, &teachers).unwrap();
    courses.extend(parse::courses(&page("selection.html"), true, &teachers).unwrap());
    let courses: Vec<_> = courses
        .into_iter()
        .map(|c| {
            let mut v = serde_json::to_value(c).unwrap();
            v.as_object_mut().unwrap().remove("weeks_mask");
            v
        })
        .collect();
    let grades:Vec<_>=parse::grades(&page("grades.html")).unwrap().into_iter().map(|g|json!({"course_name":g.course_name,"teacher":g.teacher,"term":g.term,"score":format!("{},{}",g.score_letter.unwrap_or_else(||"-".into()),g.score_value.map(|v|v.to_string()).unwrap_or_else(||"-".into())),"credits_raw":g.credits_raw,"detail":g.detail})).collect();
    let exams:Vec<_>=parse::exams(&page("exams.html")).unwrap().into_iter().map(|e|json!({"exam_type":e.exam_type,"course_name":e.course_name,"time_place_raw":e.time_place_raw,"status":e.status,"detail":e.detail})).collect();
    let public=sm2::PublicKey::from_sec1_bytes(&STANDARD.decode("BCNfHMe8JNA+F0xYeDk+8jpQX/u7gIRDbmOvxyzq5cUyaEGp88qjQT78wi2lAzmdPM6OVMEwDiqrRWqeeO+789s=").unwrap()).unwrap();
    let cipher = sm2::pke::EncryptingKey::new(public)
        .encrypt(&mut Fixed, b"fixture-Password_123!")
        .unwrap();
    println!(
        "{}",
        json!({"courses":courses,"grades":grades,"exams":exams,"profile":parse::profile(&page("profile.html"),"fixture-user").unwrap(),"card":parse::card(&page("card.json")).unwrap(),"rooms":parse::rooms(&page("rooms.html")).unwrap(),"calendar":parse::academic_weeks(&page("calendar.html")).unwrap(),"types":parse::course_types(&page("program.html")).unwrap(),"sm2_fixed":STANDARD.encode(&cipher[1..])})
    );
}
