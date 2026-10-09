plugins {
    id("com.android.application")
}

android {
    // The package of the example's Firebase app.
    namespace = "rs.pushups.example"
    compileSdk = 35

    defaultConfig {
        applicationId = "rs.pushups.example"
        minSdk = 24
        targetSdk = 35
        versionCode = 1
        versionName = "0.0.0"
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    implementation(project(":pushups"))
}
