pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "pushups-winit-example"
include(":app")

// The Android module of the `pushups` crate cargo resolved for the app's Rust library, found
// through `cargo metadata`, so the Kotlin always comes from the crate version the Rust links.
val pushups = run {
    val metadata = providers.exec {
        commandLine("cargo", "metadata", "--format-version", "1", "--manifest-path", file("../Cargo.toml").path)
    }.standardOutput.asText.get()
    @Suppress("UNCHECKED_CAST")
    val packages = (groovy.json.JsonSlurper().parseText(metadata) as Map<String, Any>)["packages"] as List<Map<String, Any>>
    packages.single { it["name"] == "pushups" }
}
// `app` depends on the AAR of the same version when `-Ppushups.aar=<Maven repository>` names
// where it is published, else on the module's sources.
gradle.extensions.extraProperties["pushupsVersion"] = pushups["version"]
val aar = providers.gradleProperty("pushups.aar").orNull
if (aar != null) {
    dependencyResolutionManagement.repositories.maven { url = uri(aar) }
} else {
    include(":pushups")
    project(":pushups").projectDir = File(pushups["manifest_path"] as String).parentFile.resolve("android")
    // Keeps the module's build output out of the crate's directory, which cargo's registry shares.
    gradle.beforeProject {
        if (path == ":pushups") layout.buildDirectory.set(rootDir.resolve("build/pushups"))
    }
}
