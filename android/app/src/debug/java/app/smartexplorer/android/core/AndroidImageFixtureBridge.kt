package app.smartexplorer.android.core

import java.nio.ByteBuffer

/** Only the instrumented debug APK contains this explicit native fixture entrance. */
internal object AndroidImageFixtureBridge {
    external fun captureOpen(width:Int,height:Int,outputWidth:Int,outputHeight:Int,kind:Int,heapOffer:Long,decoderReader:Boolean):String
    external fun captureState(session:Long):String
    external fun captureConsumeStill(session:Long,generation:Long):String
    external fun captureViewerPullBuffer(session:Long):ByteBuffer
    external fun captureReleaseDecoderReader(session:Long)
    external fun captureReleaseViewer(session:Long)
    external fun captureRetire(session:Long)
    external fun captureFinish(session:Long)
    external fun loginOpen(owner:Long,heapOffer:Long,width:Int,height:Int):Long
    external fun loginEnqueue(session:Long,frameId:Long,pattern:Int)
    external fun loginState(session:Long):String
    external fun loginFinish(session:Long)
}
