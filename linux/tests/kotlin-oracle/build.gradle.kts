plugins { kotlin("jvm") version "2.4.10"; application }
repositories { mavenCentral() }
dependencies {
    implementation("org.jetbrains.kotlinx:kotlinx-datetime:0.8.0")
    implementation("io.ktor:ktor-http:3.5.1")
    implementation("com.fleeksoft.ksoup:ksoup:0.2.6")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.8.1")
}
kotlin { jvmToolchain(21); sourceSets.main { kotlin.srcDir("build/reference") } }
application { mainClass.set("OracleKt") }
