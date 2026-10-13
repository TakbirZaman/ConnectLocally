package com.example.localconnect

import android.content.Context
import android.graphics.Point
import android.os.Build
import android.view.WindowManager

object NativeHooks {
    var transferStopped:(()->Unit)?=null
    var transferReady:((Map<String,Any>)->Unit)?=null
    var frame:((Map<String,Any>)->Unit)?=null
    var projectionReady:((Map<String,Any>)->Unit)?=null
    var projectionStopped:((String)->Unit)?=null
    var inputFailed:((String,String)->Unit)?=null
    fun geometry(context:Context):Map<String,Any> {
        val wm=context.getSystemService(WindowManager::class.java)
        val width:Int;val height:Int
        if(Build.VERSION.SDK_INT>=30){val b=wm.maximumWindowMetrics.bounds;width=b.width();height=b.height()}
        else {@Suppress("DEPRECATION") val display=wm.defaultDisplay;val point=Point();display.getRealSize(point);width=point.x;height=point.y}
        @Suppress("DEPRECATION") val rotation=wm.defaultDisplay.rotation*90
        return mapOf("width" to width,"height" to height,"rotation" to rotation,"scale" to context.resources.displayMetrics.density.toDouble())
    }
}

/** JNI shutdown does not depend on a surviving Flutter Activity/isolate. */
object RustLifecycle {
    @Volatile var generation=0L
    init {System.loadLibrary("localconnect_core")}
    private external fun shutdownNative(generation:Long)
    private external fun stopSessionNative(generation:Long,most:Long,least:Long)
    fun stopAll(epoch:Long=generation,onDone:(()->Unit)?=null) {Thread {try{shutdownNative(epoch)}finally{onDone?.let{android.os.Handler(android.os.Looper.getMainLooper()).post(it)}}}.start()}
    fun stopSession(id:String?) {if(id==null)return;val uuid=java.util.UUID.fromString(id);val epoch=generation;Thread{stopSessionNative(epoch,uuid.mostSignificantBits,uuid.leastSignificantBits)}.start()}
}
