import com.android.build.api.dsl.LibraryExtension

// The versions `dx` 0.7.10 generates for an app's Android project.
plugins {
    id("com.android.library") version "8.7.0" apply false
    id("org.jetbrains.kotlin.android") version "2.0.20" apply false
}

val ci = layout.projectDirectory
val harnessBuildDir = providers.gradleProperty("pushups.buildDir").getOrElse("build")
val jacocoVersion = providers.gradleProperty("pushups.jacoco").get()
val manifestVariant = providers.gradleProperty("pushups.manifest").getOrElse("default")

project(":pushups") {
    // Keeps build output out of the crate's own `android/` directory.
    layout.buildDirectory.set(ci.dir("$harnessBuildDir/pushups"))

    pluginManager.withPlugin("com.android.library") {
        extensions.configure<LibraryExtension> {
            defaultConfig {
                // The package of the example's Firebase app, whose client the probe compiles in.
                testApplicationId = "rs.pushups.example"
                testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
            }
            buildTypes.getByName("debug").enableAndroidTestCoverage = true
            // A test APK targeting the module's minSdk is blocked by Play Protect as built for an old Android.
            testOptions.targetSdk = 34
            testCoverage.jacocoVersion = jacocoVersion
            sourceSets.getByName("androidTest") {
                manifest.srcFile(ci.file("src/androidTest/manifests/$manifestVariant/AndroidManifest.xml"))
                java.srcDir(ci.dir("src/androidTest/kotlin"))
                jniLibs.srcDir(ci.dir("$harnessBuildDir/jniLibs"))
            }
        }
        dependencies {
            "androidTestImplementation"("androidx.test:runner:1.6.2")
            "androidTestImplementation"("androidx.test.ext:junit:1.2.1")
            "androidTestImplementation"("androidx.test.uiautomator:uiautomator:2.3.0")
            // ComponentTapActivity, an androidx Activity like wry's, for the singleTop taps.
            "androidTestImplementation"("androidx.activity:activity:1.9.3")
        }
    }
}
