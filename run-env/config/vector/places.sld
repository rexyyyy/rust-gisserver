<?xml version="1.0" encoding="UTF-8"?>
<!--
  样式：places 图层（fixtures/sample.geojson，6 个点/线/面要素）。
  与 config/vector/places.xml 同名（places.sld），map-api 启动时自动发现，
  驱动 WMS GetMap / WMTS / TMS / XYZ 的矢量栅格化出图。

  本样例演示 mapserver SLD 支持的完整符号特性（对照 GeoServer / OGC SE）：
    · 多规则 + Filter 分类：pop &gt; 800 万 → 金色五角星 + 名称标注；
      pop &lt;= 800 万 → 蓝色圆点（互斥过滤，叠加时互不干扰）
    · ElseFilter 兜底规则：缺 pop 属性的要素 → 灰色方块
    · PointSymbolizer：WellKnownName mark + Fill/Stroke/Size
    · TextSymbolizer：Label 取 name 属性 + Font(font-size/font-weight)
      + Halo 白晕 + AnchorPoint 居中
    · PolygonSymbolizer：fill / fill-opacity / stroke / stroke-width
    · LineSymbolizer：stroke + stroke-dasharray 虚线
    · MinScaleDenominator：比例尺分母 &gt;= 2,000,000（缩小到约 1:200 万以外）
      时显示城市标签，更小比例尺只留符号

  删除本文件即回退到 legacy 平面样式（默认绿填充）。
-->
<StyledLayerDescriptor version="1.1.0"
    xsi:schemaLocation="http://www.opengis.net/sld StyledLayerDescriptor.xsd"
    xmlns="http://www.opengis.net/sld"
    xmlns:se="http://www.opengis.net/se"
    xmlns:ogc="http://www.opengis.net/ogc"
    xmlns:xlink="http://www.w3.org/1999/xlink"
    xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <NamedLayer>
    <se:Name>places</se:Name>
    <UserStyle>
      <se:Name>cities</se:Name>
      <se:FeatureTypeStyle>

        <!-- 规则 1：大城市（pop > 800 万）→ 金星 + 名称标注 -->
        <se:Rule>
          <se:Name>big-city</se:Name>
          <se:Title>Major city (pop &gt; 8M)</se:Title>
          <ogc:Filter>
            <ogc:PropertyIsGreaterThan>
              <ogc:PropertyName>pop</ogc:PropertyName>
              <ogc:Literal>8000000</ogc:Literal>
            </ogc:PropertyIsGreaterThan>
          </ogc:Filter>
          <se:MinScaleDenominator>2000000</se:MinScaleDenominator>
          <se:PointSymbolizer>
            <se:Graphic>
              <se:Mark>
                <se:WellKnownName>star</se:WellKnownName>
                <se:Fill>
                  <se:SvgParameter name="fill">#ffcc00</se:SvgParameter>
                  <se:SvgParameter name="fill-opacity">0.95</se:SvgParameter>
                </se:Fill>
                <se:Stroke>
                  <se:SvgParameter name="stroke">#7a5200</se:SvgParameter>
                  <se:SvgParameter name="stroke-width">1.2</se:SvgParameter>
                </se:Stroke>
              </se:Mark>
              <se:Size>14</se:Size>
            </se:Graphic>
          </se:PointSymbolizer>
          <se:TextSymbolizer>
            <se:Label>
              <ogc:PropertyName>name</ogc:PropertyName>
            </se:Label>
            <se:Font>
              <se:SvgParameter name="font-family">Sans-Serif</se:SvgParameter>
              <se:SvgParameter name="font-size">11</se:SvgParameter>
              <se:SvgParameter name="font-weight">bold</se:SvgParameter>
            </se:Font>
            <se:LabelPlacement>
              <se:PointPlacement>
                <se:AnchorPoint>
                  <se:AnchorPointX>0.5</se:AnchorPointX>
                  <se:AnchorPointY>0.0</se:AnchorPointY>
                </se:AnchorPoint>
                <se:Displacement>
                  <se:DisplacementX>0</se:DisplacementX>
                  <se:DisplacementY>6</se:DisplacementY>
                </se:Displacement>
              </se:PointPlacement>
            </se:LabelPlacement>
            <se:Halo>
              <se:Radius>1.5</se:Radius>
              <se:Fill>
                <se:SvgParameter name="fill">#ffffff</se:SvgParameter>
              </se:Fill>
            </se:Halo>
            <se:Fill>
              <se:SvgParameter name="fill">#1a1a1a</se:SvgParameter>
            </se:Fill>
          </se:TextSymbolizer>
        </se:Rule>

        <!-- 规则 2：中小城市（pop <= 800 万）→ 蓝圆点 -->
        <se:Rule>
          <se:Name>town</se:Name>
          <se:Title>Town (pop &lt;= 8M)</se:Title>
          <ogc:Filter>
            <ogc:PropertyIsLessThanOrEqualTo>
              <ogc:PropertyName>pop</ogc:PropertyName>
              <ogc:Literal>8000000</ogc:Literal>
            </ogc:PropertyIsLessThanOrEqualTo>
          </ogc:Filter>
          <se:PointSymbolizer>
            <se:Graphic>
              <se:Mark>
                <se:WellKnownName>circle</se:WellKnownName>
                <se:Fill>
                  <se:SvgParameter name="fill">#3388cc</se:SvgParameter>
                </se:Fill>
                <se:Stroke>
                  <se:SvgParameter name="stroke">#0d3a5c</se:SvgParameter>
                  <se:SvgParameter name="stroke-width">1</se:SvgParameter>
                </se:Stroke>
              </se:Mark>
              <se:Size>9</se:Size>
            </se:Graphic>
          </se:PointSymbolizer>
        </se:Rule>

        <!-- 规则 3：兜底（缺 pop 属性）→ 灰方块 -->
        <se:Rule>
          <se:Name>unknown</se:Name>
          <ElseFilter/>
          <se:PointSymbolizer>
            <se:Graphic>
              <se:Mark>
                <se:WellKnownName>square</se:WellKnownName>
                <se:Fill>
                  <se:SvgParameter name="fill">#999999</se:SvgParameter>
                </se:Fill>
                <se:Stroke>
                  <se:SvgParameter name="stroke">#444444</se:SvgParameter>
                  <se:SvgParameter name="stroke-width">1</se:SvgParameter>
                </se:Stroke>
              </se:Mark>
              <se:Size>8</se:Size>
            </se:Graphic>
          </se:PointSymbolizer>
        </se:Rule>

      </se:FeatureTypeStyle>

      <!-- 第二个 FeatureTypeStyle：线 / 面要素（本数据集各 1 个） -->
      <se:FeatureTypeStyle>

        <!-- 面：半透明蓝填充 + 深蓝描边 -->
        <se:Rule>
          <se:Name>area</se:Name>
          <se:PolygonSymbolizer>
            <se:Fill>
              <se:SvgParameter name="fill">#2d6edc</se:SvgParameter>
              <se:SvgParameter name="fill-opacity">0.3</se:SvgParameter>
            </se:Fill>
            <se:Stroke>
              <se:SvgParameter name="stroke">#123a70</se:SvgParameter>
              <se:SvgParameter name="stroke-width">1.5</se:SvgParameter>
            </se:Stroke>
          </se:PolygonSymbolizer>
        </se:Rule>

        <!-- 线：品红虚线（stroke-dasharray 6 3） -->
        <se:Rule>
          <se:Name>corridor</se:Name>
          <se:LineSymbolizer>
            <se:Stroke>
              <se:SvgParameter name="stroke">#c0399b</se:SvgParameter>
              <se:SvgParameter name="stroke-width">2.5</se:SvgParameter>
              <se:SvgParameter name="stroke-dasharray">6 3</se:SvgParameter>
            </se:Stroke>
          </se:LineSymbolizer>
        </se:Rule>

      </se:FeatureTypeStyle>
    </UserStyle>
  </NamedLayer>
</StyledLayerDescriptor>
