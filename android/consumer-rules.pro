# The Rust cdylib resolves these members by JNI symbol name, so R8 must not strip them.
-keep class rs.pushups.** { *; }
