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

rootProject.name = "pushups-maven"

// The crate's Gradle module, published from here so the module itself carries no publishing setup.
include(":pushups")
project(":pushups").projectDir = file("../../android")
