#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|input: &str| {
    let _ = campus_core::parse::login(input);
    let _ = campus_core::parse::academic_weeks(input);
    let _ = campus_core::weeks(input);
    let _ = campus_net::media::playlist(
        input,
        &"https://lmt1.bjtu.edu.cn:8880/fixture.m3u8"
            .parse()
            .unwrap(),
    );
});
