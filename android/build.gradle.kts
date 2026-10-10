import org.gradle.api.tasks.bundling.AbstractArchiveTask

plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "rs.pushups"
    compileSdk = 34

    defaultConfig {
        minSdk = 24
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

dependencies {
    implementation(platform("com.google.firebase:firebase-bom:34.19.0"))
    implementation("com.google.firebase:firebase-messaging")
    // The newest connector whose stdlib a Kotlin 2.0 build reads, 2.0 being what `dx` 0.7 generates.
    implementation("org.unifiedpush.android:connector:3.0.10")
    implementation("androidx.core:core-ktx:1.13.1")
}

tasks.withType<AbstractArchiveTask>().configureEach {
    archiveBaseName.set("dx-native-pushups")
}
