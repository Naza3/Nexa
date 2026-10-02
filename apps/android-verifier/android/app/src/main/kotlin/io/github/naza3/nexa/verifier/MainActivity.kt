package io.github.naza3.nexa.verifier

import android.app.Activity
import android.app.AlertDialog
import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.CancellationSignal
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import android.system.Os
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.security.MessageDigest
import java.util.concurrent.Executors
import java.util.concurrent.RejectedExecutionException
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong

class MainActivity : FlutterActivity() {
    companion object {
        private val sequence=AtomicLong(0)
        private val startedActivities=AtomicInteger(0)
        private const val PICK=1401
        private const val EXPORT=1402
    }
    private val io=Executors.newSingleThreadExecutor()
    @Volatile private var epoch:String?=null
    @Volatile private var visible=false
    @Volatile private var signal:CancellationSignal?=null
    private var started=false
    private var pending:Reply?=null
    private var transferring:Reply?=null
    private var pendingEpoch:String?=null
    private var reportToken:String?=null

    /** Exactly one reply, including Activity teardown versus queued provider I/O. */
    private inner class Reply(private val delegate:MethodChannel.Result) {
        val completed=AtomicBoolean(false)
        fun success(value:Any?)=finish { delegate.success(value) }
        fun error(code:String)=finish { delegate.error(code,code,null) }
        private fun finish(action:()->Unit) {
            if(!completed.compareAndSet(false,true))return
            runOnUiThread {
                if(pending===this)pending=null
                if(transferring===this)transferring=null
                // A destroyed Flutter engine can no longer receive a reply.
                try { action() } catch (_:RuntimeException) { }
            }
        }
    }
    private fun unwrap(raw:String):JSONObject {
        val envelope=JSONObject(raw)
        if(!envelope.getBoolean("ok"))throw IllegalStateException(envelope.getJSONObject("error").getString("code"))
        return envelope.getJSONObject("value")
    }
    private fun submit(reply:Reply,work:()->Unit) {
        if(isDestroyed||reply.completed.get()){reply.error("interrupted");return}
        try { io.execute(work) } catch (_:RejectedExecutionException) {reply.error("interrupted")}
    }
    private fun launchPicker(reply:Reply,intent:Intent,code:Int) {
        try {startActivityForResult(intent,code)}
        catch (_:ActivityNotFoundException){reply.error("unsupported_source")}
        catch (_:SecurityException){reply.error("permission_denied")}
        catch (_:RuntimeException){reply.error("interrupted")}
    }
    override fun configureFlutterEngine(flutterEngine:FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        io.execute {
            try {
                val root=File(filesDir,"nexa-verifier");root.mkdirs();Os.chmod(root.path,448)
                val device=JSONObject().put("schema_version",1)
                    .put("manufacturer",Build.MANUFACTURER.take(128)).put("model",Build.MODEL.take(128))
                    .put("soc_manufacturer",if(Build.VERSION.SDK_INT>=31)Build.SOC_MANUFACTURER else JSONObject.NULL)
                    .put("soc_model",if(Build.VERSION.SDK_INT>=31)Build.SOC_MODEL else JSONObject.NULL)
                    .put("android_release",Build.VERSION.RELEASE).put("sdk_int",Build.VERSION.SDK_INT)
                    .put("security_patch",Build.VERSION.SECURITY_PATCH).put("supported_abis",JSONArray(Build.SUPPORTED_ABIS.toList()))
                epoch=unwrap(NativeVerifier.nativeBootstrap(root.canonicalPath,device.toString())).getString("host_epoch")
                runOnUiThread { if(!isDestroyed)visibility() }
            } catch (_:Throwable) { /* FRB open remains unavailable; no fake ready state. */ }
        }
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger,"io.github.naza3.nexa.verifier/platform_v1").setMethodCallHandler { call,result ->
            val reply=Reply(result)
            when(call.method) {
                "pick_candidate","export_report" -> {
                    if(pending!=null||transferring!=null){reply.error("busy");return@setMethodCallHandler}
                    val requested=call.argument<String>("epoch")
                    if(requested!=epoch){reply.error("stale_handle");return@setMethodCallHandler}
                    pending=reply;pendingEpoch=requested
                    if(call.method=="pick_candidate") {
                        launchPicker(reply,Intent(Intent.ACTION_OPEN_DOCUMENT).apply {type="*/*";addCategory(Intent.CATEGORY_OPENABLE);putExtra(Intent.EXTRA_ALLOW_MULTIPLE,true)},PICK)
                    } else {
                        reportToken=call.argument<String>("report_token")
                        launchPicker(reply,Intent(Intent.ACTION_CREATE_DOCUMENT).apply {type="text/plain";addCategory(Intent.CATEGORY_OPENABLE);putExtra(Intent.EXTRA_TITLE,"nexa-device-report.txt")},EXPORT)
                    }
                }
                "cancel_selection" -> {
                    try {unwrap(NativeVerifier.nativeCancelSelection(call.argument<String>("epoch")?:"",call.argument<String>("selection_token")?:""));reply.success(null)}
                    catch (_:Throwable){reply.error("selection_expired")}
                }
                "show_licenses" -> {
                    try {val notice=assets.open("flutter_assets/assets/THIRD_PARTY_NOTICES.txt").bufferedReader().use{it.readText()};AlertDialog.Builder(this).setTitle("Nexa 第三方许可").setMessage(notice).setPositiveButton("关闭",null).show();reply.success(null)}
                    catch (_:RuntimeException){reply.error("license_unavailable")}
                    catch (_:java.io.IOException){reply.error("license_unavailable")}
                }
                else -> result.notImplemented()
            }
        }
    }
    /** App-wide visibility; an old Activity cannot close a newer visible one. */
    private fun visibility() {
        val e=epoch?:return
        try {NativeVerifier.nativeVisibility(e,startedActivities.get()>0,sequence.incrementAndGet())}catch (_:Throwable) { }
    }
    override fun onStart(){super.onStart();visible=true;if(!started){started=true;startedActivities.incrementAndGet()};visibility()}
    private fun stopped(){visible=false;if(started){started=false;startedActivities.decrementAndGet()};visibility();signal?.cancel()}
    override fun onStop(){stopped();super.onStop()}
    override fun onDestroy(){stopped();pending?.error("interrupted");transferring?.error("interrupted");io.shutdown();super.onDestroy()}

    @Deprecated("Android activity result callback retained for Flutter embedding")
    override fun onActivityResult(requestCode:Int,resultCode:Int,data:Intent?) {
        super.onActivityResult(requestCode,resultCode,data)
        if(requestCode!=PICK&&requestCode!=EXPORT)return
        val reply=pending?:return
        pending=null;transferring=reply
        val e=pendingEpoch?:"";val token=reportToken;reportToken=null
        if(resultCode!=Activity.RESULT_OK||data==null){reply.success(if(requestCode==EXPORT)"cancelled" else null);return}
        if(requestCode==PICK) {
            val uris=mutableListOf<Uri>()
            data.clipData?.let{clip->for(i in 0 until clip.itemCount)uris.add(clip.getItemAt(i).uri)}?:data.data?.let{uris.add(it)}
            // Defer until Activity resume, but never submit to a closed executor.
            window.decorView.post { submit(reply) {registerSelection(e,uris,reply)} }
        } else {
            val uri=data.data
            if(uri==null||token==null){reply.error("report_unavailable");return}
            submit(reply){exportReport(e,token,uri,reply)}
        }
    }
    private fun registerSelection(e:String,uris:List<Uri>,reply:Reply) {
        val opened=mutableListOf<ParcelFileDescriptor>();val cancel=CancellationSignal();signal=cancel
        try {
            if(!visible||isDestroyed||reply.completed.get()){cancel.cancel();throw InterruptedException()}
            if(uris.size!=5||uris.distinct().size!=5)throw IllegalArgumentException()
            val names=mutableListOf<String>();val lengths=mutableListOf<Long>()
            for(uri in uris) {
                contentResolver.query(uri,arrayOf(OpenableColumns.DISPLAY_NAME,OpenableColumns.SIZE),null,null,null,cancel).use {c->
                    if(c==null||!c.moveToFirst()||c.isNull(0)||c.isNull(1))throw IllegalArgumentException()
                    val name=c.getString(0);if(name.toByteArray(Charsets.UTF_8).size>128)throw IllegalArgumentException()
                    names.add(name);lengths.add(c.getLong(1))
                }
                opened.add(contentResolver.openFileDescriptor(uri,"r",cancel)?:throw IllegalArgumentException())
            }
            val selected=unwrap(NativeVerifier.nativeRegisterCandidate(e,names.toTypedArray(),lengths.toLongArray(),opened.map{it.fd}.toIntArray())).getString("selection_token")
            runOnUiThread {
                if(visible&&!isDestroyed&&!reply.completed.get())reply.success(selected)
                else {try {NativeVerifier.nativeCancelSelection(e,selected)}catch (_:Throwable){};reply.error("backgrounded")}
            }
        }catch (_:Throwable){reply.error("unsupported_source")}
        finally {opened.forEach{try{it.close()}catch (_:Throwable){}};signal=null}
    }
    private fun exportReport(e:String,token:String,uri:Uri,reply:Reply) {
        try {
            val source=unwrap(NativeVerifier.nativeOpenReport(e,token))
            // Adopt first so every validation/failure path closes the transferred FD.
            ParcelFileDescriptor.adoptFd(source.getInt("fd")).use {pfd->
                val size=source.getLong("size_bytes");if(size<0||size>2097152)throw IllegalArgumentException()
                ParcelFileDescriptor.AutoCloseInputStream(pfd).use {input->
                    contentResolver.openOutputStream(uri,"wt").use {output->
                        if(output==null)throw IllegalArgumentException()
                        val hash=MessageDigest.getInstance("SHA-256");val buffer=ByteArray(65536);var total=0L
                        while(true){val n=input.read(buffer);if(n<0)break;total+=n;if(total>size)throw IllegalArgumentException();hash.update(buffer,0,n);output.write(buffer,0,n)}
                        output.flush();val actual=hash.digest().joinToString(""){"%02x".format(it)}
                        if(total!=size||actual!=source.getString("sha256"))throw IllegalArgumentException()
                    }
                }
            }
            reply.success("saved")
        }catch (_:Throwable){reply.error("report_export_failed")}
    }
}
