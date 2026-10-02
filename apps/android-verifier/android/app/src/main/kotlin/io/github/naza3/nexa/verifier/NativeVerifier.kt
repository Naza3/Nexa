package io.github.naza3.nexa.verifier
object NativeVerifier {
    init { System.loadLibrary("nexa_device_verifier") }
    @JvmStatic external fun nativeBootstrap(canonicalAppRoot: String, deviceJson: String): String
    @JvmStatic external fun nativeRegisterCandidate(epoch: String, names: Array<String>, lengths: LongArray, readFds: IntArray): String
    @JvmStatic external fun nativeCancelSelection(epoch: String, selectionToken: String): String
    @JvmStatic external fun nativeVisibility(epoch: String, visible: Boolean, lifecycleSequence: Long): String
    @JvmStatic external fun nativeOpenReport(epoch: String, reportToken: String): String
}
