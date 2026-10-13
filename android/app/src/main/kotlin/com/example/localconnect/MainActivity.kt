package com.example.localconnect

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.media.projection.MediaProjectionManager
import android.media.projection.MediaProjectionConfig
import android.provider.Settings
import android.provider.OpenableColumns
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import java.io.File
import java.util.UUID
import java.util.concurrent.Executors

class MainActivity : FlutterActivity() {
    companion object {
        private var stopCallback:((Long)->Unit)?=null
        fun stopCore(generation:Long) {android.os.Handler(android.os.Looper.getMainLooper()).post {stopCallback?.invoke(generation)}}
    }
    private lateinit var channel: MethodChannel
    private val storageEpoch=java.util.concurrent.atomic.AtomicInteger()
    @Volatile private var activeCopy:java.io.Closeable?=null
    private var pending: MethodChannel.Result? = null
    private var exportSource: File? = null
    private var notificationResult: MethodChannel.Result? = null
    private val worker = Executors.newSingleThreadExecutor()
    private var generation=0L
    private var stopResult:MethodChannel.Result?=null
    private var transferResult:MethodChannel.Result?=null
    private var projectionResult:MethodChannel.Result?=null
    private var requestedProjection:String?=null
    override fun configureFlutterEngine(engine: FlutterEngine) {
        super.configureFlutterEngine(engine)
        channel = MethodChannel(engine.dartExecutor.binaryMessenger, "localconnect/platform")
        stopCallback={epoch->channel.invokeMethod("stop",epoch)}
        val main=android.os.Handler(android.os.Looper.getMainLooper())
        NativeHooks.transferStopped={main.post{stopResult?.success(null);stopResult=null}}
        NativeHooks.transferReady={data->main.post{transferResult?.success(data);transferResult=null}}
        NativeHooks.frame={data->main.post{channel.invokeMethod("projectionFrame",data)}}
        NativeHooks.projectionStopped={ticket->main.post{channel.invokeMethod("projectionStopped",ticket)}}
        NativeHooks.inputFailed={id,error->main.post{channel.invokeMethod("inputFailed",mapOf("id" to id,"error" to error))}}
        NativeHooks.projectionReady={data->main.post{val callback=projectionResult;projectionResult=null;if(callback!=null)callback.success(data)else if(data["ok"]!=true)channel.invokeMethod("captureFailed",data)}}
        channel.setMethodCallHandler { call, result ->
            try {
                when (call.method) {
                    "bindEngine" -> {check(TransferService.available()){ "Foreground service stopped during startup; start again" };generation=(call.arguments as Number).toLong();RustLifecycle.generation=generation;TransferService.bind(generation);result.success(null)}
                    "prepareIdentity" -> {IdentityVault.prepare(this);result.success(null)}
                    "sealIdentity" -> {IdentityVault.seal(this);result.success(null)}
                    "displayGeometry" -> result.success(NativeHooks.geometry(this))
                    "accessibilitySettings" -> {startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS));result.success(null)}
                    "startProjection" -> {
                        check(projectionResult==null&&ProjectionService.ticket==null){"Stop the previous capture before requesting fresh consent"}
                        requestedProjection=call.arguments as String;projectionResult=result
                        val manager=getSystemService(MediaProjectionManager::class.java)
                        val intent=if(Build.VERSION.SDK_INT>=34)manager.createScreenCaptureIntent(MediaProjectionConfig.createConfigForDefaultDisplay())else manager.createScreenCaptureIntent()
                        startActivityForResult(intent,703)
                    }
                    "cancelStorage" -> {storageEpoch.incrementAndGet();try{activeCopy?.close()}catch(_:Exception){};pending?.success(null);pending=null;exportSource=null;result.success(null)}
                    "stopProjection" -> {requestedProjection=null;projectionResult?.success(mapOf("ok" to false,"error" to "Capture request stopped"));projectionResult=null;stopService(Intent(this,ProjectionService::class.java));RemoteAccessibilityService.authorize(null);result.success(null)}
                    "authorizeInput" -> {RemoteAccessibilityService.authorize(call.arguments as String?);result.success(null)}
                    "injectInput" -> {
                        val args=call.arguments as Map<*,*>
                        val ok=RemoteAccessibilityService.inject(args["id"] as String,args["event"] as String,args["geometry"] as Map<*,*>)
                        if(ok)result.success(null)else result.error("input_unavailable","Enable LocalConnect Accessibility manually; only taps, short swipes and editable focused text are supported.",null)
                    }
                    "startTransferService" -> {
                        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
                            if (notificationResult != null) {result.error("busy","Permission request pending",null)}
                            else {notificationResult=result; requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 700)}
                        } else startServiceForUser(result)
                    }
                    "stopTransferService" -> {if(!TransferService.available())result.success(null)else{check(stopResult==null){"Stop pending"};stopResult=result;stopService(Intent(this,TransferService::class.java))}}
                    "setBusy" -> {TransferService.setBusy(call.arguments == true); result.success(null)}
                    "importFile" -> {
                        check(pending == null) {"A file picker is already open"}; pending=result
                        startActivityForResult(Intent(Intent.ACTION_OPEN_DOCUMENT).apply {type="*/*"; addCategory(Intent.CATEGORY_OPENABLE); putExtra(Intent.EXTRA_LOCAL_ONLY,true); addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)},701)
                    }
                    "exportFile" -> {
                        check(pending == null) {"A file picker is already open"}
                        val f=File(call.arguments as String).canonicalFile
                        val allowed=File(filesDir,"received").canonicalFile
                        // path_provider support dir is filesDir on Android; allow only our receive folder.
                        require(f.parentFile == allowed && f.isFile && !f.name.endsWith(".localconnect.part")) {"Only completed files in the receive directory can be exported"}
                        exportSource=f; pending=result
                        startActivityForResult(Intent(Intent.ACTION_CREATE_DOCUMENT).apply {type="application/octet-stream"; addCategory(Intent.CATEGORY_OPENABLE); putExtra(Intent.EXTRA_TITLE,f.name)},702)
                    }
                    "batterySettings" -> {OemSettings.open(this);result.success(null)}
                    "capabilities" -> result.success(mapOf("saf" to true,"screenCaptureSupported" to true,"screenCaptureGranted" to (ProjectionService.ticket!=null),"accessibilityInput" to RemoteAccessibilityService.available(),"sdk" to Build.VERSION.SDK_INT))
                    else -> result.notImplemented()
                }
            } catch (e:Exception) {if(pending===result)pending=null;if(projectionResult===result)projectionResult=null;if(notificationResult===result)notificationResult=null;if(stopResult===result)stopResult=null;result.error("platform", e.message, null)}
        }
    }
    private fun startServiceForUser(result:MethodChannel.Result) {
        try {
            if(TransferService.available()){result.success(mapOf("ok" to true));return}
            check(transferResult==null){"Service start pending"};transferResult=result
            val i=Intent(this, TransferService::class.java)
            if(Build.VERSION.SDK_INT>=26) startForegroundService(i) else startService(i)

        } catch(e:Exception) {if(transferResult===result)transferResult=null;result.success(mapOf("ok" to false,"error" to "Keep the app visible and retry: ${e.message}"))}
    }
    override fun onRequestPermissionsResult(requestCode:Int,permissions:Array<out String>,grants:IntArray) {
        super.onRequestPermissionsResult(requestCode,permissions,grants)
        if(requestCode==700) {
            val result=notificationResult; notificationResult=null
            if(result!=null) {if(grants.firstOrNull()==PackageManager.PERMISSION_GRANTED)startServiceForUser(result)
                else result.success(mapOf("ok" to false,"error" to "Allow notification permission in app settings so active transfers and Stop remain visible."))}
        }
    }
    @Deprecated("Activity result API retained for FlutterActivity compatibility")
    override fun onActivityResult(requestCode:Int,resultCode:Int,data:Intent?) {
        super.onActivityResult(requestCode,resultCode,data)
        if(requestCode==703){
            if(projectionResult==null||requestedProjection==null)return
            if(resultCode!=Activity.RESULT_OK||data==null){projectionResult?.success(mapOf("ok" to false,"error" to "Screen capture consent denied"));projectionResult=null;return}
            try{startForegroundService(Intent(this,ProjectionService::class.java).putExtra("resultCode",resultCode).putExtra("resultData",data).putExtra("ticket",requestedProjection))}
            catch(e:Exception){projectionResult?.success(mapOf("ok" to false,"error" to "Keep the app visible and retry: ${e.message}"));projectionResult=null}
            return
        }
        if(requestCode!=701 && requestCode!=702) return
        val result=pending ?: return; pending=null
        val uri=data?.data
        if(resultCode!=Activity.RESULT_OK || uri==null) {result.success(null); return}
        val source=exportSource;exportSource=null
        val epoch=storageEpoch.get()
        worker.execute {
            try {
                if(requestCode==701) {
                    val dir=File(filesDir,"imports/${UUID.randomUUID()}");check(dir.mkdirs())
                    var name="selected-file.bin"
                    contentResolver.query(uri,arrayOf(OpenableColumns.DISPLAY_NAME),null,null,null)?.use {if(it.moveToFirst()) name=it.getString(0) ?: name}
                    name=name.replace(Regex("[\\\\/:<>\"|?*\\p{Cntrl}]"),"_").trim().take(150)
                    if(name.isEmpty() || name=="." || name=="..") name="selected-file.bin"
                    while(name.toByteArray(Charsets.UTF_8).size>160)name=name.dropLast(1)
                    val output=File(dir,name)
                    try {
                        contentResolver.openInputStream(uri).use {input -> requireNotNull(input){"Provider cannot read this document"}; output.outputStream().use {sink -> activeCopy=input;copy(input,sink,epoch);sink.fd.sync()}}
                    } catch(e:Exception) {output.delete();dir.delete();throw e}
                    runOnUiThread {result.success(output.absolutePath)}
                } else {
                    requireNotNull(source)
                    contentResolver.openOutputStream(uri,"w").use {out -> requireNotNull(out){"Provider cannot write this document"};source.inputStream().use {activeCopy=it;copy(it,out,epoch)};out.flush()}
                    // SAF providers may not offer atomic rename/fsync. Core's private verified file remains intact.
                    runOnUiThread {result.success(null)}
                }
            } catch(e:Exception) {runOnUiThread {result.error("storage","Storage permission/provider/disk error: ${e.message}",null)}}finally{activeCopy=null}
        }
    }
    private fun copy(input:java.io.InputStream,output:java.io.OutputStream,epoch:Int){
        val buffer=ByteArray(1024*1024)
        while(true){check(epoch==storageEpoch.get()&&!Thread.currentThread().isInterrupted){"Storage operation cancelled"};val count=input.read(buffer);if(count<0)break;output.write(buffer,0,count)}
        check(epoch==storageEpoch.get()){ "Storage operation cancelled" }
    }
    override fun onNewIntent(intent:Intent) {super.onNewIntent(intent);if(intent.action=="com.example.localconnect.STOP") channel.invokeMethod("stop",generation)}
    override fun onDestroy() {
        storageEpoch.incrementAndGet();try{activeCopy?.close()}catch(_:Exception){}
        transferResult?.error("closed","Activity closed during service start",null);transferResult=null;NativeHooks.transferReady=null;NativeHooks.transferStopped=null
        NativeHooks.frame=null;NativeHooks.projectionReady=null;NativeHooks.projectionStopped=null;NativeHooks.inputFailed=null;stopCallback=null
        projectionResult?.error("closed","Activity closed before consent completed",null);projectionResult=null
        RemoteAccessibilityService.authorize(null);stopService(Intent(this,ProjectionService::class.java));stopService(Intent(this,TransferService::class.java));RustLifecycle.stopAll(generation)
        worker.shutdownNow();super.onDestroy()
    }
}
