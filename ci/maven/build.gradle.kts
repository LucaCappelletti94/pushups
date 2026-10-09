import com.android.build.api.dsl.LibraryExtension

// The versions the module is built with in `ci/android`, so the AAR's Kotlin metadata stays
// readable by the Kotlin 2.0 that `dx` 0.7 apps compile with.
plugins {
    id("com.android.library") version "8.7.0" apply false
    id("org.jetbrains.kotlin.android") version "2.0.20" apply false
}

val ci = layout.projectDirectory
// The crate's version, which the AAR always carries.
val crateVersion = ci.file("../../Cargo.toml").asFile.readLines()
    .first { it.startsWith("version = ") }
    .substringAfter('"').substringBefore('"')

project(":pushups") {
    // Keeps build output out of the crate's own `android/` directory.
    layout.buildDirectory.set(ci.dir("build/pushups"))
    apply(plugin = "maven-publish")
    apply(plugin = "signing")

    pluginManager.withPlugin("com.android.library") {
        extensions.configure<LibraryExtension> {
            publishing {
                singleVariant("release") {
                    withSourcesJar()
                    withJavadocJar()
                }
            }
        }
        // Registered once AGP is applied, so it runs after AGP's own hook creates the component.
        afterEvaluate { configurePublication() }
    }
}

/** The publication of the module's release variant, its POM, the bundle repository and the signing. */
fun Project.configurePublication() {
    run {
        extensions.configure<PublishingExtension> {
            publications {
                create<MavenPublication>("release") {
                    from(components["release"])
                    groupId = "io.github.lucacappelletti94"
                    artifactId = "pushups-android"
                    version = crateVersion
                    pom {
                        name.set("pushups-android")
                        description.set(
                            "The Android module of the pushups Rust crate: FCM and UnifiedPush services handing pushes to the app's Rust. Use it with the pushups crate of the same version."
                        )
                        url.set("https://github.com/LucaCappelletti94/pushups")
                        licenses {
                            license {
                                name.set("MIT")
                                url.set("https://github.com/LucaCappelletti94/pushups/blob/main/LICENSE")
                            }
                        }
                        developers {
                            developer {
                                id.set("LucaCappelletti94")
                                name.set("Luca Cappelletti")
                                email.set("cappelletti.luca94@gmail.com")
                            }
                        }
                        scm {
                            url.set("https://github.com/LucaCappelletti94/pushups")
                            connection.set("scm:git:https://github.com/LucaCappelletti94/pushups.git")
                            developerConnection.set("scm:git:ssh://git@github.com/LucaCappelletti94/pushups.git")
                        }
                    }
                }
            }
            repositories {
                // The bundle `publish.sh` uploads to Maven Central, and the repository apps test against.
                maven {
                    name = "bundle"
                    url = uri(ci.dir("build/repo"))
                }
            }
        }

        // Signed when `publish.sh` passes the key, as Central requires. Local proofs run unsigned.
        val key = providers.environmentVariable("MAVEN_SIGNING_KEY").orNull
        if (key != null) {
            extensions.configure<SigningExtension> {
                useInMemoryPgpKeys(key, providers.environmentVariable("MAVEN_SIGNING_KEY_PASSWORD").get())
                sign(extensions.getByType<PublishingExtension>().publications["release"])
            }
        }
    }
}
