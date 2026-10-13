package com.example.localconnect

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.graphics.Path
import android.os.Bundle
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import org.json.JSONObject

/** Enabled only by the user in OS settings. No self-enabling or password extraction. */
class RemoteAccessibilityService:AccessibilityService() {
    companion object {
        private var instance:RemoteAccessibilityService?=null
        var approvedSession:String?=null;private set
        fun available()=instance!=null
        fun authorize(id:String?){approvedSession=id;instance?.pending=null}
        fun inject(id:String,event:String,expected:Map<*,*>):Boolean {
            if(id!=approvedSession)return false
            val service=instance?:return false
            val current=NativeHooks.geometry(service)
            if(listOf("width","height","rotation").any{(expected[it] as? Number)?.toInt()!=(current[it] as? Number)?.toInt()})return false
            return service.accept(id,JSONObject(event).put("geometry",JSONObject(current)))
        }
    }
    private var busy=false
    private var pending:Pair<String,JSONObject>?=null
    override fun onServiceConnected(){instance=this}
    override fun onAccessibilityEvent(event:AccessibilityEvent?) {}
    override fun onInterrupt(){revoke()}
    override fun onDestroy(){revoke();instance=null;super.onDestroy()}
    private fun revoke(){val id=approvedSession;approvedSession=null;pending=null;RustLifecycle.stopSession(id)}
    private fun point(data:JSONObject,x:String,y:String):Pair<Float,Float> {
        val px=data.getDouble(x);val py=data.getDouble(y)
        require(px.isFinite()&&py.isFinite()&&px in 0.0..1.0&&py in 0.0..1.0)
        val geometry=NativeHooks.geometry(this)
        val mapped=when(geometry["rotation"] as Int){90->Pair(py,1-px);180->Pair(1-px,1-py);270->Pair(1-py,px);else->Pair(px,py)}
        return Pair((mapped.first*((geometry["width"] as Int)-1)).toFloat(),(mapped.second*((geometry["height"] as Int)-1)).toFloat())
    }
    private fun accept(id:String,event:JSONObject):Boolean {
        if(id!=approvedSession)return false
        if(busy){pending=Pair(id,event);return true}
        return execute(id,event)
    }
    private fun execute(id:String,event:JSONObject):Boolean {
        if(id!=approvedSession)return false
        val expected=event.getJSONObject("geometry");val current=NativeHooks.geometry(this)
        if(listOf("width","height","rotation").any{expected.getInt(it)!=(current[it] as Number).toInt()})return false
        val type=event.getString("type");val data=event.getJSONObject("data")
        require(data.getString("id")==id)
        if(type=="Text") {
            val text=data.getString("text");require(text.toByteArray(Charsets.UTF_8).size<=4096&&!text.contains('\u0000'))
            val node=rootInActiveWindow?.findFocus(AccessibilityNodeInfo.FOCUS_INPUT)?:return false
            if(!node.isEditable)return false
            return node.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT,Bundle().apply{putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,text)})
        }
        val path=Path();val duration:Long
        if(type=="Tap"){val p=point(data,"x","y");path.moveTo(p.first,p.second);duration=50}
        else if(type=="Swipe"){
            val a=point(data,"x0","y0");val b=point(data,"x1","y1");duration=data.getLong("duration_ms");require(duration in 50..500)
            path.moveTo(a.first,a.second);path.lineTo(b.first,b.second)
        }else return false
        busy=true
        val gesture=GestureDescription.Builder().addStroke(GestureDescription.StrokeDescription(path,0,duration)).build()
        val accepted=dispatchGesture(gesture,object:GestureResultCallback(){
            override fun onCompleted(description:GestureDescription){next()}
            override fun onCancelled(description:GestureDescription){next()}
        },null)
        if(!accepted)busy=false
        return accepted
    }
    private fun next(){busy=false;val next=pending;pending=null;if(next!=null&&next.first==approvedSession){try{if(!execute(next.first,next.second))NativeHooks.inputFailed?.invoke(next.first,"Focused field/gesture action unavailable")}catch(e:Exception){NativeHooks.inputFailed?.invoke(next.first,e.message?:"Gesture failed")}}}
}
