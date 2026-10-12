plugins {
    id("com.android.application")
}

android {
    namespace = "com.audiorouter.app"
    compileSdk = 36

    defaultConfig {
        applicationId = "com.audiorouter.app"
        minSdk = 29
        targetSdk = 36
        versionCode = 3
        versionName = "1.0.0"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    implementation("com.plasmoverse:concentus:1.0.0")
}
