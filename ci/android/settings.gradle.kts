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

rootProject.name = "pushups-android-ci"

// The crate's Gradle module, exactly as apps include it, with its instrumented tests configured from here.
include(":pushups")
project(":pushups").projectDir = file("../../android")
